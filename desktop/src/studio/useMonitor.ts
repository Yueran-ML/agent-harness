import { useCallback, useEffect, useRef, useState } from 'react';
import type { EngineBridge } from '../bridge';
import { callEngine, type Monitor, type Rate, type TransportState } from './engine';
import { Intent, type KeyContext, type TransportCommand } from './keys';
import { nominalFps } from './timecode';

/** What a monitor has loaded: the media of the source monitor, the timeline of the timeline monitor. */
export const transportItem = (state: TransportState | null): string | null => state === null ? null : state.monitor === 'source' ? state.media : state.timeline;

/** How long the frame that the playhead was let go at, or was sent to, is shown at most before the transport tells it. */
export const LET_GO_HOLD_MS = 1000;

/** A frame that the playhead shows before the transport tells it: where it is dragged, let go, or sent. */
interface Held { frame: number; dragging: boolean; item: string | null }

/** One monitor as its transport, its keys and its pane reach it. */
export interface MonitorControls {
  monitor: Monitor;
  state: TransportState | null; rate: Rate; last: number | null; loaded: boolean;
  /**
   * The frame under the playhead as the monitor shows it, null where nothing is loaded: where it is
   * dragged, let go or sent to, until the transport tells it, else where the transport tells it is.
   * The timecode shows it, and the marks and the edits take it, as DaVinci Resolve marks the frame shown.
   */
  frame: number | null;
  /** Whether `frame` is one the transport has not told yet. */
  held: boolean;
  /** What the keys know of the transport now. */
  context: () => KeyContext;
  run: (command: TransportCommand) => void;
  seek: (frame: number, scrubbing: boolean) => void;
  /** The playhead is dragged to `frame`, or let go there; null where the drag was given up. */
  drag: (frame: number | null, dragging: boolean) => void;
  proxies: (on: boolean) => void;
}

/**
 * The transport of one monitor. Every call names the monitor, and what the keys know of it (playing,
 * the speed asked for last) is its own, so that J/K/L on one monitor do not go by the other.
 *
 * The frame that the playhead was dragged to, let go at, or sent to (a seek, a step while paused) is
 * shown at once, and taken by what reads the frame (I and O, the edits, Up and Down), until the
 * transport tells it, plays, has another item, or a second has passed: what the transport tells
 * meanwhile was on its way before, and is not the frame on the screen.
 */
export function useMonitor(bridge: Pick<EngineBridge, 'call'>, monitor: Monitor, state: TransportState | null, rate: Rate, last: number | null, onError: (error: unknown) => void): MonitorControls {
  const intent = useRef(new Intent());
  const item = transportItem(state);
  const loaded = state !== null && item !== null;
  const [held, setHeldState] = useState<Held | null>(null);
  const heldNow = useRef<Held | null>(null);
  const setHeld = useCallback((next: Held | null) => { heldNow.current = next; setHeldState(next); }, []);
  const frame = state === null || item === null ? null : held !== null && held.item === item ? held.frame : state.frame;
  const now = useRef({ rate, last, onError, item, frame, state });
  now.current = { rate, last, onError, item, frame, state };
  useEffect(() => { if (state) intent.current.told(state); else intent.current = new Intent(); }, [state]);
  // A frame held is let go once the transport is there, plays, or has gone elsewhere.
  useEffect(() => {
    if (held === null) return;
    if (state === null || item === null || held.item !== item) { setHeld(null); return; }
    if (!held.dragging && (state.frame === held.frame || state.playing)) setHeld(null);
  }, [held, state, item, setHeld]);
  useEffect(() => {
    if (held === null || held.dragging) return undefined;
    const timer = setTimeout(() => setHeld(null), LET_GO_HOLD_MS);
    return () => clearTimeout(timer);
  }, [held, setHeld]);
  const ask = useCallback((method: TransportCommand['method'] | 'transport.proxies', params?: Record<string, unknown>) => {
    void callEngine(bridge, method, { ...params, monitor }).catch(error => now.current.onError(error));
  }, [bridge, monitor]);
  const hold = useCallback((to: number, dragging = false) => setHeld({ frame: to, dragging, item: now.current.item }), [setHeld]);
  const run = useCallback((command: TransportCommand) => {
    const { frame: at, last: end, state: told } = now.current;
    const paused = !intent.current.playing && told?.playing !== true;
    intent.current.sent(command);
    if (command.method === 'transport.seek') hold(command.params.frame);
    else if (command.method === 'transport.step' && paused && at !== null) {
      // A step while paused goes from the frame shown; one while playing, from where the transport stops.
      const to = Math.max(0, at + command.params.frames);
      hold(end === null ? to : Math.min(end, to));
    } else setHeld(null);
    ask(command.method, command.params);
  }, [ask, hold, setHeld]);
  const seek = useCallback((to: number, scrubbing: boolean) => {
    // A seek of a drag that the playhead shows already leaves the drag held.
    if (!(scrubbing && heldNow.current?.dragging)) hold(to);
    ask('transport.seek', { frame: to, scrubbing });
  }, [ask, hold]);
  const drag = useCallback((to: number | null, dragging: boolean) => { if (to === null) setHeld(null); else hold(to, dragging); }, [hold, setHeld]);
  const proxies = useCallback((on: boolean) => ask('transport.proxies', { on }), [ask]);
  const context = useCallback(() => ({ playing: intent.current.playing, speed: intent.current.speed, fps: nominalFps(now.current.rate), last: now.current.last }), []);
  return { monitor, state, rate, last, loaded, frame, held: frame !== null && held !== null && held.item === item, context, run, seek, drag, proxies };
}
