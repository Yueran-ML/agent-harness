import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { Timeline } from '../engine';
import { clipLocations } from './selection';

const NONE: readonly string[] = [];

/**
 * What a page that shows Retime Controls on its timeline gives it: the clips whose controls are
 * open, which the page keeps (not the project), and what opens or closes them. `onToggle` must
 * stay the same call.
 */
export interface RetimeControlsState {
  open: readonly string[];
  /**
   * Show the Retime Controls of clips, or put them away where all of those clips show them already
   * (Clip › Retime Controls, Ctrl+R: Reference Manual 20, p. 1074). A clip that has no speed to
   * change, a title or a solid colour, is left out.
   */
  onToggle: (clips: readonly string[]) => void;
}

/** Whether a clip may show Retime Controls: it is there, and is cut from media. (A still has no speed either; the engine says so where an edit is asked of it.) */
const retimable = (timeline: Timeline, id: string): boolean => clipLocations(timeline).get(id)?.clip.source.kind === 'media';

/**
 * The clips whose Retime Controls stay open on a timeline: one that is gone has none. The list
 * itself where all of them stay.
 */
export function openRetimeControls(timeline: Timeline | null, open: readonly string[]): readonly string[] {
  if (open.length === 0) return open;
  const kept = timeline ? open.filter(id => retimable(timeline, id)) : [];
  return kept.length === open.length ? open : kept;
}

/**
 * The Retime Controls that are open on the timeline of a page, by the ids of their clips: kept by
 * the page, as DaVinci Resolve keeps them with the timeline's view and not with the edit (Reference
 * Manual 20, p. 1074). Another timeline, or another session, begins with none open, and a clip that
 * is gone has none.
 *
 * What it gives stays the same until controls are opened or closed, so a timeline with none open
 * is given nothing new as the project changes.
 */
export function useRetimeControls(timeline: Timeline | null, session: number): RetimeControlsState {
  const [open, setOpen] = useState<readonly string[]>(NONE);
  const now = useRef(timeline);
  now.current = timeline;
  useEffect(() => setOpen(NONE), [session, timeline?.id]);
  useEffect(() => {
    // With none open there is nothing to look for, and nothing is set.
    if (open.length === 0) return;
    const kept = openRetimeControls(timeline, open);
    if (kept !== open) setOpen(kept);
  }, [timeline, open]);
  const onToggle = useCallback((clips: readonly string[]) => {
    const shown = now.current;
    const able = shown ? clips.filter(id => retimable(shown, id)) : [];
    if (able.length === 0) return;
    setOpen(old => (able.every(id => old.includes(id)) ? old.filter(id => !able.includes(id)) : [...old, ...able.filter(id => !old.includes(id))]));
  }, []);
  return useMemo(() => ({ open, onToggle }), [open, onToggle]);
}
