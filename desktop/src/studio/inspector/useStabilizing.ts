import { useCallback, useEffect, useRef, useState } from 'react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { callEngine, DEFAULT_STABILIZE, type Clip, type Timeline, type TrackEnded, type TrackProgress } from '../engine';
import { useTrackJob } from '../useTrackJob';
import { refusalOf } from './refusal';

/** What is said of the analysis of a stabilizer, and the clip that it is said of. */
export interface StabilizeSaid { clip: string; text: string }

/** The analysis of the stabilizer of a clip, as the Inspector drives it (see {@link useStabilizing}). */
export interface Stabilizing {
  /** Whether an analysis of this Inspector goes on. */
  running: boolean;
  /** Whether one is asked for and not answered yet. */
  asking: boolean;
  status: StabilizeSaid | null;
  /** Measure the clip shown, for the Method of its stabilizer. */
  start: () => void;
  stop: () => void;
}

/**
 * The Stabilize button of the Inspector of the Edit and Cut pages (Reference Manual 20, pp. 633–634 and 1064–1065;
 * PROTOCOL.md, "The stabilizer"), driven as the trackers of the Color and Fusion pages are (`useTrackJob`):
 *
 * - Stabilize asks the engine to measure the motion of the whole picture of the one clip shown (`track.stabilize`), for the
 *   Method that the Inspector shows: its stabilizer's, or Perspective where it has none. The engine writes the analysis once,
 *   at the end, as one step to undo, with the Method that it measured for; a measure that is stopped or fails writes nothing.
 * - What is said is said of the clip measured: how far the measure went, how it ended, or why the engine would not measure.
 * - The measure is stopped as another clip is shown and as the Inspector goes (the page left, another tab, the panel closed),
 *   as the Fusion page stops a track of its Tracker.
 * - A measure that the Inspector did not start is not taken over: its news is like that of a track of a window, and a press of
 *   Stabilize while one goes on is refused as busy, in words.
 */
export function useStabilizing({ bridge, session, revision, timeline, clip, locked }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>;
  session: number;
  revision: number;
  timeline: Timeline | null;
  /** The one clip shown; null where several are. */
  clip: Clip | null;
  /** Whether it lies on a locked track: nothing of it is measured. */
  locked: boolean;
}): Stabilizing {
  const words = useWords();
  const [status, setStatus] = useState<StabilizeSaid | null>(null);
  // The clip that the measure asked for or going on is of; null where none is.
  const [target, setTarget] = useState<string | null>(null);
  const [asking, setAsking] = useState(false);
  const now = useRef({ bridge, revision, timeline, clip, locked, target, asking, words });
  now.current = { bridge, revision, timeline, clip, locked, target, asking, words };
  const aim = useCallback((next: string | null) => { now.current.target = next; setTarget(next); }, []);
  const ask = useCallback((on: boolean) => { now.current.asking = on; setAsking(on); }, []);
  // Whether the Inspector is shown: a measure answered once it went is stopped at once.
  const live = useRef(true);

  /** How far the measure went: `done` frames of `total`. */
  const progressed = useCallback((done: number, total: number) => {
    const { target: aimed, words: said } = now.current;
    if (aimed === null) return;
    const text = said.hint.studio.inspector.stabilize.analysing.replace('{done}', () => String(done)).replace('{total}', () => String(total));
    setStatus({ clip: aimed, text });
  }, []);

  const { running, current, start: startJob, stop: stopJob } = useTrackJob({
    bridge, session, method: 'track.stabilize',
    adopts: () => false,
    onProgress: (told: TrackProgress) => progressed(told.done, told.total),
    onEnded: (told: TrackEnded) => {
      const { target: aimed, words: said } = now.current;
      const stabilize = said.hint.studio.inspector.stabilize;
      let text: string;
      if (told.phase === 'done') {
        text = stabilize.done.replace('{frames}', () => String(told.frames));
        if (told.lost > 0) text = `${text} ${stabilize.lost.replace('{lost}', () => String(told.lost))}`;
      } else if (told.phase === 'stopped') text = told.reason === 'stop' ? stabilize.stopped : stabilize.changed;
      else text = told.reason !== null && Object.hasOwn(stabilize.failed, told.reason) ? stabilize.failed[told.reason as keyof typeof stabilize.failed] : stabilize.failed.other;
      if (aimed !== null) setStatus({ clip: aimed, text });
      aim(null);
    },
  });

  /** Why the engine would not measure, in the words of the Inspector, and a failure of another kind as the Inspector says one. */
  const refused = useCallback((failure: unknown) => {
    const { target: aimed, words: said } = now.current;
    ask(false);
    if (aimed !== null) setStatus({ clip: aimed, text: refusalOf(failure, said.hint.studio, { ...said.hint.studio.inspector.stabilize.failed }).text });
    aim(null);
  }, [aim, ask]);

  const start = useCallback(() => {
    const { bridge: caller, revision: project, timeline: on, clip: of, locked: off, asking: asked } = now.current;
    if (on === null || of === null || off || asked || current() !== null) return;
    setStatus(null);
    aim(of.id);
    ask(true);
    startJob(caller, { timeline: on.id, clip: of.id, revision: project, method: (of.stabilize ?? DEFAULT_STABILIZE).method }, refused, (answer) => {
      ask(false);
      // A measure that is answered once the Inspector went goes no further, as one going on as it goes.
      if (!live.current) void callEngine(caller, 'track.stop', { job: answer.job }).catch(() => {});
      else progressed(0, answer.last - answer.first + 1);
    });
  }, [aim, ask, current, progressed, refused, startJob]);

  const stop = useCallback(() => stopJob(now.current.bridge), [stopJob]);

  // Another clip shown, or several, stops the measure of the clip that was shown.
  const clipId = clip?.id ?? null;
  const going = running !== null;
  useEffect(() => {
    if (!going || target === null) return;
    if (target !== clipId) stop();
  }, [going, target, clipId, stop]);

  // The Inspector gone (the page left, another tab, the panel closed): the measure goes no further.
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
      stopJob(now.current.bridge);
    };
  }, [stopJob]);

  return { running: going, asking, status, start, stop };
}
