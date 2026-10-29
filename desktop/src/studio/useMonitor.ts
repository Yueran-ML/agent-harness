import { useCallback, useEffect, useRef } from 'react';
import type { EngineBridge } from '../bridge';
import { callEngine, type Monitor, type Rate, type TransportState } from './engine';
import { Intent, type KeyContext, type TransportCommand } from './keys';
import { nominalFps } from './timecode';

/** What a monitor has loaded: the media of the source monitor, the timeline of the timeline monitor. */
export const transportItem = (state: TransportState | null): string | null => state === null ? null : state.monitor === 'source' ? state.media : state.timeline;

/** One monitor as its transport, its keys and its pane reach it. */
export interface MonitorControls {
  monitor: Monitor;
  state: TransportState | null; rate: Rate; last: number | null; loaded: boolean;
  /** What the keys know of the transport now. */
  context: () => KeyContext;
  run: (command: TransportCommand) => void;
  seek: (frame: number, scrubbing: boolean) => void;
  proxies: (on: boolean) => void;
}

/**
 * The transport of one monitor. Every call names the monitor, and what the keys know of it (playing,
 * the speed asked for last) is its own, so that J/K/L on one monitor do not go by the other.
 */
export function useMonitor(bridge: Pick<EngineBridge, 'call'>, monitor: Monitor, state: TransportState | null, rate: Rate, last: number | null, onError: (error: unknown) => void): MonitorControls {
  const intent = useRef(new Intent());
  const now = useRef({ rate, last, onError });
  now.current = { rate, last, onError };
  useEffect(() => { if (state) intent.current.told(state); else intent.current = new Intent(); }, [state]);
  const ask = useCallback((method: TransportCommand['method'] | 'transport.proxies', params?: Record<string, unknown>) => {
    void callEngine(bridge, method, { ...params, monitor }).catch(error => now.current.onError(error));
  }, [bridge, monitor]);
  const run = useCallback((command: TransportCommand) => { intent.current.sent(command); ask(command.method, command.params); }, [ask]);
  const seek = useCallback((frame: number, scrubbing: boolean) => ask('transport.seek', { frame, scrubbing }), [ask]);
  const proxies = useCallback((on: boolean) => ask('transport.proxies', { on }), [ask]);
  const context = useCallback(() => ({ playing: intent.current.playing, speed: intent.current.speed, fps: nominalFps(now.current.rate), last: now.current.last }), []);
  return { monitor, state, rate, last, loaded: transportItem(state) !== null, context, run, seek, proxies };
}
