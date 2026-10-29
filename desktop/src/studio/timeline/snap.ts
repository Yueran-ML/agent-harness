import type { Timeline } from '../engine';

export interface Marks { in: number | null; out: number | null }
/** Moving clips cannot snap to their own old edges. Marks use an exclusive out frame. */
export function snapCandidates(timeline: Timeline, excluded: readonly string[], playhead: number, marks: Marks): number[] {
  const skip = new Set(excluded);
  const points = new Set([0, playhead]);
  if (marks.in !== null) points.add(marks.in);
  if (marks.out !== null) points.add(marks.out);
  for (const track of timeline.tracks) for (const item of track.kind === 'subtitle' ? track.cues : track.clips) if (!skip.has(item.id)) {
    points.add(item.start); points.add(item.start + item.length);
  }
  return [...points].sort((a, b) => a - b);
}

/** Add delta to all anchors, and use the closest candidate within tolerance CSS pixels. */
export function snap(delta: number, anchors: readonly number[], candidates: readonly number[], scale: number, tolerance = 8): { delta: number; point: number | null } {
  let best = tolerance / scale;
  let correction = 0;
  let point: number | null = null;
  for (const anchor of anchors) {
    const at = anchor + delta;
    let low = 0;
    let high = candidates.length;
    while (low < high) { const mid = (low + high) >>> 1; if (candidates[mid] < at) low = mid + 1; else high = mid; }
    for (const index of [low - 1, low]) {
      if (index < 0 || index >= candidates.length) continue;
      const candidate = candidates[index];
      const distance = Math.abs(candidate - at);
      if (distance < best || distance === best && (point === null || candidate < point)) { best = distance; point = candidate; correction = candidate - at; }
    }
  }
  return { delta: delta + correction, point };
}
