import type { Clip, ClipTrack, Timeline } from '../engine';

export interface Selection { ids: string[]; anchor: string | null }
export const EMPTY_SELECTION: Selection = { ids: [], anchor: null };
export interface Located { clip: Clip; track: ClipTrack; index: number }
export function clipLocations(timeline: Timeline): Map<string, Located> {
  const locations = new Map<string, Located>();
  for (const track of timeline.tracks) if (track.kind !== 'subtitle') track.clips.forEach((clip, index) => locations.set(clip.id, { clip, track, index }));
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
