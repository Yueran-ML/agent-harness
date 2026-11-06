import type { ClipTrack, Timeline } from '../engine';
import type { CopiedLane } from '../timeline/commands';
import { ownerKey, ownerOf } from './automation';

/** A level that may be pasted to any other (p. 3914): a fader's, or a send's. */
export const isLevelPath = (path: string) => path === 'fader_db' || /^send\.[^.]+\.level_db$/.test(path);

/**
 * The levels that the automation clipboard may be pasted to on the selected tracks and buses
 * (those of the first, or of every audio track and bus where none is selected): the fader, and
 * each send's level.
 */
export function levelPathsOf(timeline: Timeline, selected: readonly string[]): string[] {
  const owners = selected.length ? selected.map(ownerOf) : [
    ...timeline.tracks.filter(track => track.kind === 'audio').map(track => ({ track: track.id })),
    ...(timeline.buses ?? []).map(bus => ({ bus: bus.id })),
  ];
  const sends = new Set<string>();
  for (const of of owners) {
    const strip = 'track' in of
      ? (timeline.tracks.find(track => track.kind === 'audio' && track.id === of.track) as ClipTrack | undefined)?.strip
      : 'bus' in of ? timeline.buses?.find(bus => bus.id === of.bus)?.strip : undefined;
    for (const send of strip?.sends ?? [])
      sends.add(`send.${send.bus}.level_db`);
  }
  return ['fader_db', ...[...sends].sort()];
}

/**
 * The lanes that Paste writes: the clipboard's, each to the selected track that takes the place
 * of its own (one source to every selected track, or the sources to as many tracks in order),
 * and, where `level` names a level, the level of each source (its fader's, or else its first
 * send's) pasted to that level instead, the rest of the clipboard left out.
 */
export function pastedLanes(clipboard: readonly CopiedLane[], selected: readonly string[], level: string): CopiedLane[] {
  const sources = [...new Set(clipboard.map(lane => ownerKey(lane.of)))];
  const placed = selected.length
    ? selected.flatMap((key, index) => clipboard
      .filter(lane => ownerKey(lane.of) === sources[sources.length === 1 ? 0 : index])
      .map(lane => ({ ...lane, of: ownerOf(key) })))
    : [...clipboard];
  if (!level)
    return placed;
  const byOwner = new Map<string, CopiedLane>();
  for (const lane of placed.filter(each => isLevelPath(each.path))) {
    const key = ownerKey(lane.of), kept = byOwner.get(key);
    if (!kept || (kept.path !== 'fader_db' && lane.path === 'fader_db'))
      byOwner.set(key, lane);
  }
  return [...byOwner.values()].map(lane => ({ ...lane, path: level }));
}
