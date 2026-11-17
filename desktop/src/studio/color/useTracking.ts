import { useCallback, useEffect, useRef, useState } from 'react';
import { keyOf } from '../../app/keys';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { EngineError, type TrackComponents, type TrackDirection } from '../engine';
import { editingAllowed, inStudio } from '../keys';
import { useTrackJob, type RunningTrack } from '../useTrackJob';
import { DEFAULT_COMPONENTS, trackerKey } from './tracker';
import type { EditedNode } from './useNodeEdit';

/** What the Color page keeps of tracking while the Tracker palette comes and goes (see {@link useTracking}). */
export interface Tracking {
  /** The five boxes of the transforms to track (p. 3235). */
  components: TrackComponents;
  setComponents: (change: (old: TrackComponents) => TrackComponents) => void;
  tracker: 'cloud' | 'point';
  setTracker: (kind: 'cloud' | 'point') => void;
  running: RunningTrack | null;
  /** What the palette says: the frame reached, how the track ended, or why something was refused. */
  status: string | null;
  setStatus: (status: string | null) => void;
  start: (direction: TrackDirection) => void;
  stop: () => void;
  /** Say why the engine refused a track or an edit of the palette, or hand any other failure on. */
  refused: (failure: unknown) => void;
}

/**
 * The tracking of the Color page (Reference Manual 20, p. 3231; PROTOCOL.md, "Window tracking"), kept by the page and
 * not by the Tracker palette, as DaVinci's Color › Tracker menu is the page's:
 *
 * - The keys of that menu work wherever on the page, whichever palette is open, for the window selected in the Window
 *   palette: Track Forward Ctrl+T, Track Reverse Alt+T, Track Stop Ctrl+Alt+T, One Frame Forward Alt+→ and One Frame
 *   Reverse Alt+←.
 * - The track going on, the boxes and the kind of tracker stay while the palette is closed, so Stop works after
 *   another palette was opened, and the palette shows the track again when it is opened.
 * - What the engine tells of its track keeps this in step (`useTrackJob`, which the Fusion page's Tracker shares):
 *   `track.progress` (sent only to the connection that started the track) makes a track this page did not know of its
 *   own, as after the page was opened again while one went on, where it is a window's and not the patterns' of a Tracker
 *   (whose news names the patterns it follows); and `track.state` ends it.
 */
export function useTracking({ edited, bridge, window, onSeek }: {
  edited: EditedNode | null;
  bridge: Pick<EngineBridge, 'onEvent'>;
  /** The window selected in the Window palette. */
  window: string | null;
  /** Take the viewer to a frame of the timeline, as the track reaches it. */
  onSeek?: (frame: number) => void;
}): Tracking {
  const hints = useWords().hint.studio.colorPage.tracker;
  const [components, setComponents] = useState<TrackComponents>(DEFAULT_COMPONENTS);
  const [tracker, setTracker] = useState<'cloud' | 'point'>('cloud');
  const [status, setStatus] = useState<string | null>(null);
  const now = useRef({ edited, window, components, tracker, hints });
  now.current = { edited, window, components, tracker, hints };

  const { running, current, start: startJob, stop: stopJob } = useTrackJob({
    bridge, session: edited?.session, onSeek,
    // A track of the patterns of a Tracker (of the Fusion page) tells the patterns it follows: it is no window's.
    adopts: told => !Array.isArray(told.following),
    onProgress: told => setStatus(now.current.hints.tracking(told.frame, told.done, told.total)),
    onEnded: (told) => {
      const said = now.current.hints;
      if (told.phase === 'done') setStatus(said.done(told.frames));
      else if (told.phase === 'stopped') setStatus(told.reason === 'stop' ? said.stopped : said.changed);
      else setStatus(said.failed[(told.reason ?? 'other') as keyof typeof said.failed] ?? said.failed.other);
    },
  });

  const refused = useCallback((failure: unknown) => {
    const failed = now.current.hints.failed;
    if (failure instanceof EngineError) {
      const reason = (failure.data as { reason?: string } | undefined)?.reason;
      setStatus(reason !== undefined && reason in failed ? failed[reason as keyof typeof failed] : failure.message);
    } else now.current.edited?.onFailure(failure);
  }, []);

  const start = useCallback((direction: TrackDirection) => {
    const { edited: context, window: chosen, components: boxes, tracker: kind } = now.current;
    if (context === null || chosen === null || current() !== null || context.node.kind !== 'corrector') return;
    if (!(context.node.params.windows ?? []).some(each => each.id === chosen)) return;
    setStatus(null);
    startJob(context.bridge, {
      grade: context.target, node: context.node.id, window: chosen, revision: context.revision, frame: context.frame,
      direction, tracker: kind, components: boxes,
    }, refused);
  }, [current, startJob, refused]);

  const stop = useCallback(() => {
    const context = now.current.edited;
    if (context === null) return;
    stopJob(context.bridge);
  }, [stopJob]);

  // The keys of the Color › Tracker menu (p. 3231), on the whole page.
  useEffect(() => {
    const down = (event: globalThis.KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key)) return;
      const asked = trackerKey(key);
      if (asked === null) return;
      if (asked === 'stop' ? current() === null : now.current.window === null) return;
      event.preventDefault();
      if (asked === 'stop') stop(); else start(asked);
    };
    globalThis.addEventListener('keydown', down);
    return () => globalThis.removeEventListener('keydown', down);
  }, [current, start, stop]);

  return { components, setComponents, tracker, setTracker, running, status, setStatus, start, stop, refused };
}
