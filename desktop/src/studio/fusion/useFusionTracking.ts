import { useCallback, useEffect, useRef, useState } from 'react';
import { keyOf } from '../../app/keys';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { covers } from '../color/clips';
import { trackerKey } from '../color/tracker';
import { callEngine, type Clip, type FusionNode, type FusionTrackerPattern, type Media, type Timeline, type TrackDirection, type TrackEnded, type TrackProgress } from '../engine';
import { frameOfKey, keyFrameOf } from '../inspector/clipKeys';
import { refusalOf } from '../inspector/refusal';
import { editingAllowed, inStudio } from '../keys';
import { timecode } from '../timecode';
import { useTrackJob } from '../useTrackJob';
import { patternName } from './tracker';

/** The kinds of node that the page tracks: the patterns of a Tracker, and the region of a Planar Tracker. */
export type TrackedKind = 'tracker' | 'planar_tracker';
/** Whether a node is one that the page tracks. */
export const isTrackedNode = (node: FusionNode | null): node is Extract<FusionNode, { kind: TrackedKind }> => node !== null
  && (node.kind === 'tracker' || node.kind === 'planar_tracker');

/**
 * Whether a node is the one that the track going on follows, which nothing else may change while it goes: the node that the track
 * was started for, or, where the page took over a track that it did not start (which is of the patterns of a Tracker), any Tracker.
 */
export function isBeingTracked(tracking: Pick<FusionTracking, 'running' | 'node'> | undefined, node: FusionNode | null): boolean {
  if (tracking === undefined || !tracking.running || !isTrackedNode(node)) return false;
  return tracking.node === node.id || (tracking.node === null && node.kind === 'tracker');
}

/** What is said of tracking, and the Tracker that it is said of: null where the page took over a track that it did not start. */
export interface TrackSaid {
  node: string | null;
  text: string;
  /** Of how a track ended: the revision that its last write made, after which a change of the project (undo among them) makes it old. */
  revision?: number;
}

/** What a track was asked for: the clip, the node, and its kind, which says what is said of the track. */
interface Target { clip: string; node: string; kind: TrackedKind }

/** The tracking of the patterns of a Tracker and of the region of a Planar Tracker, as the Fusion page keeps it (see {@link useFusionTracking}). */
export interface FusionTracking {
  /** Whether a track goes on. */
  running: boolean;
  /** The Tracker whose patterns the track going on follows; null where the page took over a track that it did not start. */
  node: string | null;
  status: TrackSaid | null;
  /** Track the enabled patterns of the Tracker chosen from the playhead, in a direction. */
  start: (direction: TrackDirection) => void;
  stop: () => void;
}

/**
 * The tracking of the patterns of a Tracker on the Fusion page (Reference Manual 20, pp. 2773–2784; PROTOCOL.md, "The patterns
 * of a Tracker"), and of the region of a Planar Tracker (pp. 1760–1766 and 2749–2758; "The region of a Planar Tracker"), driven
 * as the Color page drives the tracking of a window (`useTrackJob`):
 *
 * - A track follows every enabled pattern of the one node chosen, a Tracker, or the region of a Planar Tracker chosen, from the
 *   frame of the keys of the clip that the playhead shows (`keyFrameOf`); a playhead outside the clip is said so and nothing is
 *   asked. The engine writes the paths, or the path of the plane, as it goes, as one step to undo, and the viewer is taken to
 *   each frame that it reaches, as on the Color page; the clip tracked is pinned (`onStarted`), so that it stays the clip shown
 *   while the playhead goes over a clip on a track above.
 * - What is said is said of the node tracked: the frame reached and the patterns (of a Tracker) or the frames (of a Planar
 *   Tracker) lost while it goes; how it ended, with the frame of the timeline where each pattern was lost (`frameOfKey`, as the
 *   paths are shown), or how many frames of the plane were lost; why the engine would not track.
 * - The track is stopped as the clip or the node chosen is another, and as the page is left.
 * - The keys of the Color › Tracker menu (p. 3231) are taken while the node editor or the Inspector has the focus and a
 *   Tracker or a Planar Tracker is the node chosen: Track Forward Ctrl+T, Track Reverse Alt+T, One Frame Forward Alt+→, One
 *   Frame Reverse Alt+←; Stop Ctrl+Alt+T wherever on the page while a track goes on. None of them is a key of the page, its node
 *   editor or the transport (which take no Alt, and no Ctrl+T).
 * - A track of the patterns of a Tracker of this connection that the page did not start, as after it was loaded again, is
 *   taken as its own, of whichever Tracker is chosen; a track of a window (of the Color page) is not, and neither is a track of
 *   a Planar Tracker, whose news cannot be told from that of a window.
 */
export function useFusionTracking({ bridge, session, revision, timeline, clip, media, node, frame, locked, keys, onSeek, onStarted }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>;
  session: number;
  revision: number;
  timeline: Timeline | null;
  /** The clip whose composition the page shows. */
  clip: Clip | null;
  media: readonly Media[];
  /** The one node that is chosen, or null where none or several are. */
  node: FusionNode | null;
  /** The playhead, in frames of the timeline; null while it is not known. */
  frame: number | null;
  /** Whether the clip lies on a locked track: nothing of it is tracked. */
  locked: boolean;
  /** Whether the keys of tracking are the page's now: the node editor or the Inspector has the focus. */
  keys: boolean;
  /** Take the viewer to a frame of the timeline. */
  onSeek: (frame: number) => void;
  /** A track of a Tracker of the clip `clip` was started. */
  onStarted?: (clip: string) => void;
}): FusionTracking {
  const words = useWords();
  const [status, setStatus] = useState<TrackSaid | null>(null);
  // What the track asked for or going on was started for: the clip, the node and its kind. Null where none was, or the page took one over.
  const [target, setTarget] = useState<Target | null>(null);
  const now = useRef({ bridge, revision, timeline, clip, media, node, frame, locked, keys, target, words, onStarted });
  now.current = { bridge, revision, timeline, clip, media, node, frame, locked, keys, target, words, onStarted };
  const aim = useCallback((next: Target | null) => { now.current.target = next; setTarget(next); }, []);
  // Whether the page is shown: a track answered once it was left is stopped at once.
  const live = useRef(true);

  /** A frame of the timeline as the transport writes it. */
  const timecodeOf = (at: number): string => {
    const on = now.current.timeline;
    return on === null ? String(at) : timecode(at, on.rate);
  };
  /** The frame of the timeline that shows a frame of the keys of the clip, as the transport writes it. */
  const timecodeOfKey = (keyFrame: number): string => {
    const { timeline: on, clip: of, media: all } = now.current;
    return on === null || of === null ? String(keyFrame) : timecode(frameOfKey(of, on, all, keyFrame), on.rate);
  };
  /** Where each pattern followed was lost, named as the Inspector names it. */
  const lostLines = (told: TrackEnded): string[] => {
    const { node: chosen, target: aimed, words: said } = now.current;
    const patterns: readonly FusionTrackerPattern[] = chosen !== null && chosen.kind === 'tracker' && (aimed === null || aimed.node === chosen.id)
      ? chosen.params.trackers : [];
    const lines: string[] = [];
    for (const { tracker, lost } of told.trackers ?? []) {
      const frames: readonly number[] = Array.isArray(lost) ? lost : [];
      const name = patternName(patterns[tracker], tracker, said.label.studio.fusionPage.settings.tracker);
      for (const at of frames) lines.push(said.hint.studio.fusionPage.track.lostAt.replace('{name}', () => name).replace('{timecode}', () => timecodeOfKey(at)));
    }
    return lines;
  };

  const { running, current, start: startJob, stop: stopJob } = useTrackJob({
    bridge, session, onSeek,
    // A track of the patterns of a Tracker tells the patterns that it follows; one of a window does not.
    adopts: told => Array.isArray(told.following),
    onProgress: (told: TrackProgress) => {
      const { timeline: on, clip: of, media: all, target: aimed, words: said } = now.current;
      const at = told.timeline_frame ?? (on !== null && of !== null ? frameOfKey(of, on, all, told.frame) : told.frame);
      // Of a Tracker the patterns lost so far, of a Planar Tracker the frames.
      const pattern = aimed?.kind === 'planar_tracker' ? said.hint.studio.fusionPage.planarTrack.tracking : said.hint.studio.fusionPage.track.tracking;
      setStatus({
        node: aimed?.node ?? null,
        text: pattern.replace('{timecode}', () => timecodeOf(at)).replace('{done}', () => String(told.done))
          .replace('{total}', () => String(told.total)).replace('{lost}', () => String(told.lost)),
      });
    },
    onEnded: (told: TrackEnded) => {
      const { target: aimed, words: said } = now.current;
      const track = said.hint.studio.fusionPage.track;
      const planar = aimed?.kind === 'planar_tracker';
      const failed: Record<string, string> = planar ? { ...track.failed, ...said.hint.studio.fusionPage.planarTrack.failed } : { ...track.failed };
      let text: string;
      if (told.phase === 'done') {
        text = planar ? said.hint.studio.fusionPage.planarTrack.done.replace('{frames}', () => String(told.frames)).replace('{lost}', () => String(told.lost))
          : track.done.replace('{frames}', () => String(told.frames));
      } else if (told.phase === 'stopped') text = told.reason === 'stop' ? track.stopped : track.changed;
      else text = told.reason !== null && Object.hasOwn(failed, told.reason) ? failed[told.reason] : track.failed.other;
      // Where each pattern of a Tracker was lost, as far as the track went.
      if (told.phase !== 'failed' && !planar) text = [text, ...lostLines(told)].join(' ');
      setStatus({ node: aimed?.node ?? null, text, revision: told.revision });
      aim(null);
    },
  });

  /** Why the engine would not track, in the words of the Fusion page, and a failure of another kind as the Inspector says one. */
  const refused = useCallback((failure: unknown) => {
    const { target: aimed, words: said } = now.current;
    const page = said.hint.studio.fusionPage;
    const reasons = aimed?.kind === 'planar_tracker' ? { ...page.track.failed, ...page.planarTrack.failed } : { ...page.track.failed };
    setStatus({ node: aimed?.node ?? null, text: refusalOf(failure, said.hint.studio, reasons).text });
    aim(null);
  }, [aim]);

  const start = useCallback((direction: TrackDirection) => {
    const { bridge: caller, revision: project, timeline: on, clip: of, media: all, node: chosen, frame: playhead, locked: off, words: said } = now.current;
    if (on === null || of === null || !isTrackedNode(chosen) || off || current() !== null) return;
    if (playhead === null || !covers(of, playhead)) {
      setStatus({ node: chosen.id, text: said.hint.studio.fusionPage.track.failed.frame });
      return;
    }
    setStatus(null);
    aim({ clip: of.id, node: chosen.id, kind: chosen.kind });
    // The patterns of a Tracker are its `fusion` target, the region of a Planar Tracker its `planar` one.
    const named = { timeline: on.id, clip: of.id, node: chosen.id };
    const asked = chosen.kind === 'tracker' ? { fusion: named } : { planar: named };
    startJob(caller, { ...asked, revision: project, frame: keyFrameOf(of, on, all, playhead), direction },
      refused, (answer) => {
        // A track that is answered once the page was left goes no further, as one going on as it is left.
        if (!live.current) void callEngine(caller, 'track.stop', { job: answer.job }).catch(() => {});
        else now.current.onStarted?.(of.id);
      });
  }, [aim, current, refused, startJob]);

  const stop = useCallback(() => stopJob(now.current.bridge), [stopJob]);

  // Another clip or another node chosen stops the track of the Tracker that was chosen.
  const clipId = clip?.id ?? null;
  const nodeId = node?.id ?? null;
  const going = running !== null;
  useEffect(() => {
    if (!going || target === null) return;
    if (target.clip !== clipId || target.node !== nodeId) stop();
  }, [going, target, clipId, nodeId, stop]);

  // How a track ended is said until the project changes after it: an undo of the track, or any other change, makes it old.
  const endedAt = status?.revision;
  useEffect(() => {
    if (endedAt !== undefined && revision > endedAt) setStatus(null);
  }, [endedAt, revision]);

  // The page left, or made anew for another timeline or engine: the track goes no further.
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
      stopJob(now.current.bridge);
    };
  }, [stopJob]);

  // The keys of the Color › Tracker menu, for the Tracker chosen.
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key)) return;
      const asked = trackerKey(key);
      if (asked === null) return;
      if (asked === 'stop') {
        if (current() === null) return;
        event.preventDefault();
        stop();
        return;
      }
      const { keys: ours, node: chosen } = now.current;
      if (!ours || !isTrackedNode(chosen)) return;
      event.preventDefault();
      start(asked);
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, [current, start, stop]);

  return { running: going, node: going ? target?.node ?? null : null, status, start, stop };
}
