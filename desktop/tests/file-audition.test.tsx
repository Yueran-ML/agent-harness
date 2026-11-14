// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { FileTransport, useFileAudition } from '../src/studio/inspector/FileAudition';
import { FileWaveforms } from '../src/studio/inspector/FileWaveforms';
import { MediaFile } from '../src/studio/media/MediaFile';
import type { ClipTrack, Media, PageMethods } from '../src/studio/engine';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

type Status = PageMethods['audio.audition'];
const settle = () => act(async () => { for (let i = 0; i < 35; i++) await Promise.resolve(); });
const tick = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });
const source: Media = { ...media, facts: { ...media.facts, duration: { ticks: 10, scale: 1 },
  audio: { rate: 48000, channels: 2, codec: 'pcm', streams: [{ index: 1, channels: 2, layout: 'stereo' }] } } };

beforeEach(() => {
  likeABrowser(); vi.useFakeTimers(); vi.stubGlobal('PointerEvent', MouseEvent);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(400);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
    left: 0, right: 400, top: 0, bottom: 54, width: 400, height: 54, x: 0, y: 0, toJSON() {},
  });
});
afterEach(async () => { cleanup(); await settle(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

function wire() {
  let id = 0;
  let status: Status = { audition: null, playing: false, position: null, error: null };
  let deferred: (() => Promise<void>) | null = null;
  let failed = false;
  const call = vi.fn<EngineBridge['call']>(async (method, params) => {
    if (method !== 'audio.audition') return { ok: true, result: { rate: 10, channels: 2, peaks: btoa('\0'.repeat(400)) } };
    if (params?.action === 'play') {
      if (deferred) { const wait = deferred; deferred = null; await wait(); }
      if (failed) { failed = false; throw new Error('device failed'); }
      status = { audition: ++id, playing: true, position: params.at as Status['position'], error: null };
    } else if (params?.action === 'stop' && params.audition === status.audition) status = { ...status, playing: false };
    return { ok: true, result: { ...status } };
  });
  return {
    call, onEvent: () => () => {},
    requests: () => call.mock.calls.filter(([method]) => method === 'audio.audition').map(([, params]) => params!),
    status: () => status,
    end: () => { status = { ...status, playing: false }; },
    fail: () => { failed = true; },
    defer: () => {
      let release!: () => void;
      const waiting = new Promise<void>(resolve => { release = resolve; });
      deferred = () => waiting;
      return release;
    },
  };
}

function Harness({ bridge, mediaId = 'm1', session = 1 }: { bridge: ReturnType<typeof wire>; mediaId?: string; session?: number }) {
  const audition = useFileAudition(bridge, session, { kind: 'media', media: mediaId }, 0, 10);
  return <section onKeyDown={audition.keyDown}>
    <button onClick={() => audition.locate(4, 1)}>Lane</button>
    <input aria-label="Level" />
    <FileTransport audition={audition} />
  </section>;
}

it('seeks while stopped, solos only during playback and Stop then Play restores all channels', async () => {
  const bridge = wire(); render(<Harness bridge={bridge} />);
  fireEvent.click(screen.getByText('Lane')); await settle();
  expect(bridge.requests()).toEqual([]);
  fireEvent.click(screen.getByText('Play source')); await settle();
  expect(bridge.requests().at(-1)).toMatchObject({ action: 'play', channel: null, at: { ticks: 4_000_000, scale: 1_000_000 } });
  fireEvent.click(screen.getByText('Lane')); await settle();
  expect(bridge.requests().at(-1)).toMatchObject({ action: 'play', channel: 1 });
  fireEvent.click(screen.getByText('Stop source')); await settle();
  expect(bridge.requests().at(-1)).toEqual({ action: 'stop', audition: 2 });
  fireEvent.click(screen.getByText('Play source')); await settle();
  expect(bridge.requests().at(-1)).toMatchObject({ action: 'play', channel: null });
});

it('handles Space locally without hijacking numeric/text input or issuing a main-monitor command', async () => {
  const bridge = wire(); render(<Harness bridge={bridge} />);
  const mainKey = vi.fn(); window.addEventListener('keydown', mainKey);
  fireEvent.keyDown(screen.getByLabelText('Level'), { code: 'Space', key: ' ' }); await settle();
  expect(bridge.requests()).toEqual([]);
  fireEvent.keyDown(screen.getByText('Lane'), { code: 'Space', key: ' ' }); await settle();
  expect(bridge.status().playing).toBe(true);
  fireEvent.keyDown(screen.getByText('Lane'), { code: 'Space', key: ' ', repeat: true }); await settle();
  expect(bridge.requests()).toHaveLength(1);
  fireEvent.keyDown(screen.getByText('Stop source'), { code: 'Space', key: ' ' }); await settle();
  expect(bridge.status().playing).toBe(false);
  expect(mainKey).not.toHaveBeenCalled(); window.removeEventListener('keydown', mainKey);
  expect(bridge.call.mock.calls.every(([method]) => method === 'audio.audition')).toBe(true);
});

it('serializes a delayed old selection and cleans up its exact id without stopping the new selection', async () => {
  const bridge = wire(), release = bridge.defer();
  const view = render(<Harness bridge={bridge} />);
  fireEvent.click(screen.getByText('Play source')); await settle();
  view.rerender(<Harness bridge={bridge} mediaId="m2" />);
  fireEvent.click(screen.getByText('Play source')); await settle();
  expect(bridge.requests().filter(request => request.action === 'play')).toHaveLength(1);
  release(); await settle();
  expect(bridge.requests().filter(request => request.action === 'play').map(request => request.target)).toEqual([
    { kind: 'media', media: 'm1' }, { kind: 'media', media: 'm2' },
  ]);
  expect(bridge.requests()).toContainEqual({ action: 'stop', audition: 1 });
  expect(bridge.status()).toMatchObject({ audition: 2, playing: true });
  view.unmount(); await settle();
  expect(bridge.status().playing).toBe(false);
  expect(bridge.requests().at(-1)).toEqual({ action: 'stop', audition: 2 });
});

it('stops a late Play after unmount and stops polling at EOF or after a device failure', async () => {
  const bridge = wire(), release = bridge.defer();
  const view = render(<Harness bridge={bridge} />);
  fireEvent.click(screen.getByText('Play source')); await settle();
  view.unmount(); release(); await settle();
  expect(bridge.status().playing).toBe(false);
  render(<Harness bridge={bridge} />);
  bridge.fail(); fireEvent.click(screen.getByText('Play source')); await settle();
  expect(screen.getByRole('alert').textContent).toContain('Could not play source audio');
  fireEvent.click(screen.getByText('Play source')); await settle();
  bridge.end(); await tick(100);
  expect(screen.queryByRole('alert')).toBeNull();
  expect((screen.getByText('Play source') as HTMLButtonElement).disabled).toBe(false);
  const calls = bridge.requests().length; await tick(1000);
  expect(bridge.requests()).toHaveLength(calls);
});

it('coalesces skim positions, leaves bounded audition and keeps modifier selection silent', async () => {
  const bridge = wire(), onApply = vi.fn(async () => true);
  render(<MediaFile bridge={bridge} session={7} media={[source]} onApply={onApply} />); await settle();
  const lane = screen.getByRole('button', { name: 'Channel 2' });
  for (let x = 1; x <= 100; x++) fireEvent.pointerMove(lane, { clientX: x });
  expect(bridge.requests()).toEqual([]);
  await tick(60);
  expect(bridge.requests()).toEqual([{ action: 'play', target: { kind: 'media', media: 'm1' }, channel: 1,
    skim: true, at: { ticks: 2_500_000, scale: 1_000_000 } }]);
  expect(bridge.call.mock.calls.at(-1)?.[2]).toEqual({ expectedSession: 7 });
  fireEvent.pointerLeave(lane); await settle();
  expect(bridge.status().playing).toBe(false);
  const count = bridge.requests().length;
  fireEvent.pointerMove(lane, { clientX: 200, ctrlKey: true });
  fireEvent.click(lane, { detail: 1, clientX: 200, shiftKey: true }); await tick(200);
  expect(bridge.requests()).toHaveLength(count);
  expect(onApply).not.toHaveBeenCalled();
});

it('auditions the last media selection and composite restores the enabled channel mix', async () => {
  const bridge = wire();
  render(<MediaFile bridge={bridge} session={1} media={[source, { ...source, id: 'last' }]} onApply={async () => true} />);
  await settle(); fireEvent.click(screen.getByText('Play source')); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Channel 1' }), { detail: 1, clientX: 80 }); await settle();
  expect(bridge.requests().at(-1)).toMatchObject({ channel: 0, target: { kind: 'media', media: 'last' } });
  const count = bridge.requests().length;
  fireEvent.click(screen.getByRole('button', { name: 'Multiple Clips' }), { detail: 1, clientX: 240, ctrlKey: true }); await settle();
  expect(bridge.requests()).toHaveLength(count);
  fireEvent.click(screen.getByRole('button', { name: 'Multiple Clips' }), { detail: 1, clientX: 240 }); await settle();
  expect(bridge.requests().at(-1)).toMatchObject({ channel: null, at: { ticks: 6_000_000, scale: 1_000_000 } });
});

it('suspends audition while timeline channel changes are unapplied', async () => {
  const bridge = wire(), value = clip('audio');
  const track: ClipTrack = { id: 'a1', name: 'Audio', kind: 'audio', enabled: true, locked: false, format: 'stereo', clips: [value], transitions: [] };
  const props = { bridge, session: 1, clip: value, track, media: source, timeline: timeline([]), selected: [], onSelect: () => {} };
  const view = render(<FileWaveforms {...props} />); await settle();
  fireEvent.click(screen.getByText('Play source')); await settle();
  expect(bridge.status().playing).toBe(true);
  view.rerender(<FileWaveforms {...props} unapplied />); await settle();
  expect(bridge.status().playing).toBe(false);
  expect(screen.getByText('Apply channel changes before auditioning them.')).toBeTruthy();
  expect((screen.getByText('Play source') as HTMLButtonElement).disabled).toBe(true);
  const count = bridge.requests().length;
  fireEvent.keyDown(screen.getByRole('button', { name: 'Channel 1' }), { code: 'Space' });
  fireEvent.pointerMove(screen.getByRole('button', { name: 'Channel 1' }), { clientX: 80 }); await tick(200);
  expect(bridge.requests()).toHaveLength(count);
});

it('maps reversed retimed timeline waveforms to original source time without submitting clip processing', async () => {
  const bridge = wire(), onSelect = vi.fn();
  const value = clip('audio', 100, 50, { source: { kind: 'media', media: 'm1', in: { ticks: 2, scale: 1 } },
    speed: { num: 2, den: 1 }, reverse: true, gain_db: 12, pitch: { semitones: 7, cents: 0 } });
  const track: ClipTrack = { id: 'a1', name: 'Audio', kind: 'audio', enabled: true, locked: false, format: 'stereo', clips: [value], transitions: [] };
  render(<FileWaveforms bridge={bridge} session={1} clip={value} track={track} media={source} timeline={timeline([])} selected={[]} onSelect={onSelect} />);
  await settle(); fireEvent.click(screen.getByText('Play source')); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Channel 2' }), { detail: 1, clientX: 100 }); await settle();
  expect(bridge.requests().at(-1)).toEqual({ action: 'play', target: { kind: 'clip', timeline: 't1', clip: 'audio' },
    channel: 1, skim: false, at: { ticks: 5_000_000, scale: 1_000_000 } });
  expect(onSelect).toHaveBeenCalledWith(1, false);
  expect(screen.queryByRole('button', { name: 'Composite waveform' })).toBeNull();
});

it('a keyboard press on a lane solos it from the current position instead of jumping to the start', async () => {
  const bridge = wire(), value = clip('audio');
  const track: ClipTrack = { id: 'a1', name: 'Audio', kind: 'audio', enabled: true, locked: false, format: 'stereo', clips: [value], transitions: [] };
  render(<FileWaveforms bridge={bridge} session={1} clip={value} track={track} media={source} timeline={timeline([])}
    selected={[]} onSelect={() => {}} />);
  await settle(); fireEvent.click(screen.getByText('Play source')); await settle();
  const played = bridge.requests().at(-1) as { at: { ticks: number } };
  // A click activated by Enter carries detail 0 and no pointer position.
  fireEvent.click(screen.getByRole('button', { name: 'Channel 2' }), { detail: 0, clientX: 0 }); await settle();
  expect(bridge.requests().at(-1)).toMatchObject({ action: 'play', channel: 1, at: played.at });
});
