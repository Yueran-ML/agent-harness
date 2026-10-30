import type { Clip, Op, Timeline } from '../engine';
import { clipLocations } from '../timeline/selection';
import type { Build } from './useCommit';

/** What a set_clip may set. */
export type ClipPatch = Omit<Extract<Op, { op: 'set_clip' }>, 'op' | 'timeline' | 'id'>;

/** The value that all have, or null where they differ (or there are none). */
export function common<T, U>(items: readonly T[], get: (item: T) => U): U | null {
  if (items.length === 0) return null;
  const first = get(items[0]);
  const same = JSON.stringify(first);
  return items.every(item => JSON.stringify(get(item)) === same) ? first : null;
}

const differs = (clip: Clip, patch: ClipPatch) => Object.entries(patch)
  .some(([key, value]) => value !== undefined && JSON.stringify(clip[key as keyof Clip]) !== JSON.stringify(value));

/**
 * One transaction that sets each of the clips that are still there as `change` says, worked out
 * from each as it is when it is sent; clips that it would not change are left out, and so are clips
 * on locked tracks, whose settings cannot be changed (Reference Manual 20, p. 762).
 */
export function setClips(ids: readonly string[], change: (clip: Clip, timeline: Timeline) => ClipPatch | null): Build {
  return timeline => {
    const locations = clipLocations(timeline);
    const ops: Op[] = [];
    for (const id of ids) {
      const located = locations.get(id);
      if (!located || located.track.locked) continue;
      const patch = change(located.clip, timeline);
      if (patch && differs(located.clip, patch)) ops.push({ op: 'set_clip', timeline: timeline.id, id, ...patch });
    }
    return ops;
  };
}

/** For each clip that has a transition at an edge, which: the engine allows no fade there. */
export function transitionEdges(timeline: Timeline): Map<string, { in: boolean; out: boolean }> {
  const edges = new Map<string, { in: boolean; out: boolean }>();
  const at = (id: string) => { let edge = edges.get(id); if (!edge) { edge = { in: false, out: false }; edges.set(id, edge); } return edge; };
  for (const track of timeline.tracks) if (track.kind !== 'subtitle') for (const transition of track.transitions) {
    at(transition.left).out = true;
    at(transition.right).in = true;
  }
  return edges;
}
