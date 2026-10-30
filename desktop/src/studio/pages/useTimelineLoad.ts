import { useEffect, useRef } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Timeline } from '../engine';
import type { MonitorControls } from '../useMonitor';

/**
 * The timeline monitor plays the timeline that a page shows: loaded once as it is chosen, not again
 * at each edit of it, nor where the monitor has it already. Returns the timeline that it plays.
 */
export function useTimelineLoad(bridge: Pick<EngineBridge, 'call'>, selected: Timeline | null, timelineMonitor: MonitorControls, session: number, onFailure: (failure: unknown) => void): string | null {
  const playing = timelineMonitor.state?.monitor === 'timeline' ? timelineMonitor.state.timeline : null;
  const requested = useRef<{ session: number; timeline: string } | null>(null);
  const now = useRef(onFailure);
  now.current = onFailure;
  useEffect(() => {
    if (selected === null) return;
    const asked = requested.current;
    if (playing === selected.id || (asked !== null && asked.session === session && asked.timeline === selected.id)) {
      requested.current = { session, timeline: selected.id };
      return;
    }
    requested.current = { session, timeline: selected.id };
    void callEngine(bridge, 'transport.load', { monitor: 'timeline', timeline: selected.id }).catch(failure => now.current(failure));
  }, [bridge, selected?.id, playing, session]);
  return playing;
}
