import type { Clip, ClipTrack, Timeline } from '../engine';

export interface Selection { ids: string[]; anchor: string | null }
export const EMPTY_SELECTION: Selection = { ids: [], anchor: null };
export interface Located { clip: Clip; track: ClipTrack; index: number }
const located = new WeakMap<Timeline, ReadonlyMap<string, Located>>();
/**
 * Where each clip of a timeline is, by its id. A timeline from the engine is not changed in place
 * (an edit makes a new one), so this is worked out once for each, not at every pointer move.
 */
export function clipLocations(timeline: Timeline): ReadonlyMap<string, Located> {
  let locations = located.get(timeline);
  if (!locations) {
    const made = new Map<string, Located>();
    for (const track of timeline.tracks) if (track.kind !== 'subtitle') track.clips.forEach((clip, index) => made.set(clip.id, { clip, track, index }));
    located.set(timeline, made);
    locations = made;
  }
  return locations;
}

export function linkedClips(timeline: Timeline, ids: readonly string[], linked: boolean): string[] {
  const locations = clipLocations(timeline);
  const selected = [...new Set(ids)].filter(id => locations.has(id));
  if (!linked) return selected;
  const groups = new Set(selected.flatMap(id => locations.get(id)!.clip.link ?? []));
  for (const [id, { clip }] of locations) if (clip.link && groups.has(clip.link) && !selected.includes(id)) selected.push(id);
  return selected;
}

/**
 * The clips that a drag of `ids` moves, trims or slides, with those linked to them (`linked`), split
 * by their tracks: `movable` on tracks that are not locked, and `lockedPartners`, clips linked to
 * them on locked tracks. As in DaVinci Resolve, a clip on a locked track stays where it is and the
 * rest of its group moves (Reference Manual 20, pp. 762, 769): a move names the movable ones only,
 * and the engine leaves the others alone. A copy made with Alt is refused where a copy of a locked
 * partner would go on a locked track (`copy_locked`), so it is told of them apart.
 */
export function movableClips(timeline: Timeline, ids: readonly string[], linked: boolean): { movable: string[]; lockedPartners: string[] } {
  const locations = clipLocations(timeline);
  const movable: string[] = [];
  const lockedPartners: string[] = [];
  for (const id of linkedClips(timeline, ids, linked)) (locations.get(id)!.track.locked ? lockedPartners : movable).push(id);
  return { movable, lockedPartners };
}

/**
 * Track 1 of the Cut page, as DaVinci Resolve calls it: the first video track, which holds the
 * primary shots and ripples by itself (Reference Manual 20, pp. 509, 584–590); and its clips, with
 * the clips linked to them (their sound), which go with them. Null where there is no video track.
 */
export function primaryTrack(timeline: Timeline): { track: string; clips: ReadonlySet<string> } | null {
  const track = timeline.tracks.find(each => each.kind === 'video');
  if (!track || track.kind !== 'video') return null;
  const ids = new Set(track.clips.map(clip => clip.id));
  const groups = new Set(track.clips.flatMap(clip => clip.link ?? []));
  for (const [id, { clip }] of clipLocations(timeline)) if (clip.link && groups.has(clip.link)) ids.add(id);
  return { track: track.id, clips: ids };
}

export function selectClips(timeline: Timeline, before: Selection, id: string, options: { linked: boolean; toggle?: boolean; range?: boolean }): Selection {
  const locations = clipLocations(timeline);
  const clicked = locations.get(id);
  if (!clicked || clicked.track.locked) return before;
  const anchor = before.anchor ? locations.get(before.anchor) : undefined;
  let selected = [id];
  if (options.range && anchor?.track.id === clicked.track.id) {
    selected = clicked.track.clips.slice(Math.min(anchor.index, clicked.index), Math.max(anchor.index, clicked.index) + 1).map(clip => clip.id);
  }
  const group = linkedClips(timeline, selected, options.linked);
  const ids = options.toggle
    ? !options.range && group.some(each => before.ids.includes(each)) ? before.ids.filter(each => !group.includes(each)) : [...new Set([...before.ids, ...group])]
    : group;
  return { ids, anchor: options.range ? before.anchor ?? id : id };
}
