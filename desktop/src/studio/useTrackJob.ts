import { useCallback, useEffect, useRef, useState } from 'react';
import type { EngineBridge } from '../bridge';
import { callEngine, type PageMethods, type TrackEnded, type TrackProgress } from './engine';

/** A track going on in the engine, as a page knows it: its job, and the engine's session it is of. */
export interface RunningTrack { job: string; session?: number }

/**
 * The method that starts a job of the tracker: `track.start` (a window of the Color page, the patterns of a Tracker of the
 * Fusion page) or `track.stabilize` (the analysis of the stabilizer of a clip of the Edit and Cut pages). Each is answered at
 * once with its job, told by `track.progress` and `track.state`, and stopped by `track.stop`.
 */
export type TrackStart = 'track.start' | 'track.stabilize';

/** A track of the engine as a page drives it (see {@link useTrackJob}). */
export interface TrackJob {
  running: RunningTrack | null;
  /** The track going on as the engine last told it, read at once: news taken over since the last render is in it. */
  current: () => RunningTrack | null;
  /**
   * Ask the engine for a track (the page's method, `track.start` unless it says another, with `params`) on the session that
   * the page is on. Nothing while a track goes on. What the engine refuses goes to `refused`, and its answer to `started`.
   */
  start: (bridge: Pick<EngineBridge, 'call'>, params: Record<string, unknown>, refused: (failure: unknown) => void, started?: (answer: PageMethods[TrackStart]) => void) => void;
  /** Stop the track going on (`track.stop`): it ends between frames, keeping what it tracked. Nothing where none goes on. */
  stop: (bridge: Pick<EngineBridge, 'call'>) => void;
}

/**
 * One track of the engine at a time, as a page drives it (PROTOCOL.md, "Window tracking", "The patterns of a Tracker" and "The
 * stabilizer"): the windows of the Color page, the patterns of a Tracker of the Fusion page and the analysis of the stabilizer
 * of a clip of the Edit and Cut pages are tracked the same way.
 *
 * - The track is asked for by `method` (`track.start`, or `track.stabilize`), answered at once with the job, and `track.stop`
 *   ends it between frames.
 * - What the engine tells of the track keeps the page in step: `track.progress`, sent only to the connection that started the
 *   track, takes the viewer to the frame reached (`onSeek`), and `track.state` ends it. News of another job, or of the engine
 *   of another session, is not this page's.
 * - A track of this connection that the page did not start, as after the page was loaded again while one went on, is taken
 *   as its own from its `track.progress`, where `adopts` says that it is of what this page tracks.
 */
export function useTrackJob({ bridge, session, method = 'track.start', adopts, onSeek, onProgress, onEnded }: {
  bridge: Pick<EngineBridge, 'onEvent'>;
  /** The engine's session that the page is on; undefined where it is not known, and news of any is taken. */
  session: number | undefined;
  /** The method that asks for a track: `track.start` where it is left out. */
  method?: TrackStart;
  /** Whether a track that the page did not start is of what this page tracks: a window, or the patterns of a Tracker. */
  adopts: (told: TrackProgress) => boolean;
  /** Take the viewer to a frame of the timeline, as the track reaches it. */
  onSeek?: (frame: number) => void;
  onProgress: (told: TrackProgress) => void;
  onEnded: (told: TrackEnded) => void;
}): TrackJob {
  const [running, setRunning] = useState<RunningTrack | null>(null);
  const now = useRef({ running, session, method, adopts, onSeek, onProgress, onEnded });
  now.current = { running, session, method, adopts, onSeek, onProgress, onEnded };

  const current = useCallback(() => now.current.running, []);

  const start = useCallback((caller: Pick<EngineBridge, 'call'>, params: Record<string, unknown>, refused: (failure: unknown) => void,
    started?: (answer: PageMethods[TrackStart]) => void) => {
    const { running: going, session: on, method: asked } = now.current;
    if (going !== null) return;
    void callEngine(caller, asked, params, on === undefined ? undefined : { expectedSession: on })
      .then((answer) => {
        // Known at once, before the page is drawn again: the news of a short track may follow its answer that closely.
        const mine = now.current.running ?? { job: answer.job, session: on };
        now.current.running = mine;
        setRunning(old => old ?? mine);
        started?.(answer);
      })
      .catch(refused);
  }, []);

  const stop = useCallback((caller: Pick<EngineBridge, 'call'>) => {
    const going = now.current.running;
    if (going === null) return;
    void callEngine(caller, 'track.stop', { job: going.job }).catch(() => {});
  }, []);

  // What the engine tells of the track: the frame reached, shown in the viewer, and how it ended.
  useEffect(() => bridge.onEvent(event => {
    if (event.kind !== 'notification') return;
    const { running: going, session: on } = now.current;
    if (event.method === 'track.progress') {
      const told = event.params as TrackProgress;
      if (typeof told?.job !== 'string') return;
      if (going === null) {
        // A track of this connection that the page did not start, or started before it was loaded again.
        if (on !== undefined && event.session !== on) return;
        if (!now.current.adopts(told)) return;
        const adopted = { job: told.job, session: event.session };
        now.current.running = adopted;
        setRunning(adopted);
      } else if (told.job !== going.job || (going.session !== undefined && event.session !== going.session)) return;
      now.current.onProgress(told);
      if (told.timeline_frame !== null && told.timeline_frame !== undefined) now.current.onSeek?.(told.timeline_frame);
    } else if (event.method === 'track.state') {
      const told = event.params as TrackEnded;
      if (going === null || told?.job !== going.job || (going.session !== undefined && event.session !== going.session)) return;
      now.current.running = null;
      setRunning(null);
      now.current.onEnded(told);
    }
  }), [bridge]);

  return { running, current, start, stop };
}
