import { useCallback, useEffect, useMemo, useState } from 'react';
import type { Timeline } from '../engine';
import type { KeyTracksState } from './ClipKeyMarks';
import { openTracks } from './keyTrack';

const NONE: readonly string[] = [];

/**
 * The keyframe tracks that are open on the timeline of a page, by the ids of their clips: kept by
 * the page, not by the project, and as many as were opened (DaVinci Resolve Reference Manual 20,
 * p. 1109). Another timeline, or another session, begins with none open. A clip that is gone, or
 * that loses its last key (by an edit here, by someone else, or by Undo), has no keyframe track any
 * more: it closes, and stays closed where the keys come back.
 *
 * What it gives stays the same until a track is opened or closed, so a timeline with none open is
 * given nothing new as the project changes.
 */
export function useKeyTracks(timeline: Timeline | null, session: number): KeyTracksState {
  const [open, setOpen] = useState<readonly string[]>(NONE);
  useEffect(() => setOpen(NONE), [session, timeline?.id]);
  useEffect(() => {
    // With none open there is nothing to look for, and nothing is set.
    if (open.length === 0) return;
    const kept = openTracks(timeline, open);
    if (kept !== open) setOpen(kept);
  }, [timeline, open]);
  const onToggle = useCallback((clip: string) => setOpen(old => (old.includes(clip) ? old.filter(each => each !== clip) : [...old, clip])), []);
  return useMemo(() => ({ open, onToggle }), [open, onToggle]);
}
