import { useCallback, useEffect, useRef, useState } from 'react';
import { keyOf } from '../../app/keys';
import type { EngineBridge } from '../../bridge';
import { callEngine, type TimelineTransportState } from '../engine';
import { editingAllowed, inStudio } from '../keys';
import { SCRUB_EVERY_MS } from '../Transport';

/** Same leading/trailing throttle as the source scrubber; a release cancels queued approximations. */
export class Scrub {
  private at = -Infinity;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private waiting: number | null = null;
  constructor(private readonly seek: (frame: number, scrubbing: boolean) => void) {}
  drag(frame: number): void {
    this.waiting = frame;
    const wait = SCRUB_EVERY_MS - (Date.now() - this.at);
    if (wait <= 0 && this.timer === null) this.send();
    else if (this.timer === null) this.timer = setTimeout(() => { this.timer = null; this.send(); }, Math.max(0, wait));
  }
  private send(): void {
    const frame = this.waiting;
    this.waiting = null;
    if (frame !== null) { this.at = Date.now(); this.seek(frame, true); }
  }
  finish(frame: number): void { this.cancel(); this.seek(frame, false); }
  cancel(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null; this.waiting = null; this.at = -Infinity;
  }
}

/**
 * Timeline › Audio Scrubbing (Reference Manual 20, p. 731): whether dragging the playhead plays the
 * sound under it, on at first. It is the timeline monitor's (`transport.scrub_audio`, phase 3 T18),
 * kept from page to page as Bypass Color is: a page that shows the timeline monitor shows it as the
 * state of the transport tells it and turns it with Shift+S. The Color page leaves Shift+S to its
 * nodes (it adds a node before the one selected there).
 */
export function useAudioScrubbing({ bridge, session, loaded, state, onFailure }: {
  bridge: Pick<EngineBridge, 'call'>;
  session: number;
  /** Whether a timeline is loaded in the monitor. */
  loaded: boolean;
  state: TimelineTransportState | null;
  onFailure: (failure: unknown) => void;
}) {
  const told = state?.scrub_audio ?? true;
  const [on, setOn] = useState(told);
  const now = useRef({ bridge, session, loaded, onFailure, on });
  now.current = { bridge, session, loaded, onFailure, on };

  useEffect(() => { setOn(told); }, [told]);

  const toggle = useCallback(() => {
    const at = now.current;
    if (!at.loaded) return;
    const next = !at.on;
    setOn(next);
    void callEngine(at.bridge, 'transport.scrub_audio', { monitor: 'timeline', on: next }, { expectedSession: at.session })
      .catch((failure) => { setOn(!next); at.onFailure(failure); });
  }, []);

  // Shift+S (p. 731).
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key) || key.repeat || key.ctrl || key.meta || key.alt || !key.shift) return;
      const letter = /^Key([A-Z])$/.exec(key.code ?? '')?.[1]?.toLowerCase() ?? (key.key.length === 1 ? key.key.toLowerCase() : '');
      if (letter !== 's') return;
      event.preventDefault();
      toggle();
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, [toggle]);

  return { scrubAudio: on, toggleScrubAudio: toggle };
}
