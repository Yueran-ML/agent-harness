import type { GroupControl, Op, Timeline, TrackGroup } from '../engine';
export const GROUP_CONTROLS: GroupControl[] = ['editing', 'fader', 'solo', 'mute', 'sends', 'panning', 'automation'];
export const GROUP_DEFAULTS_KEY = 'fairlight.group_defaults';
export const allGroup = (timeline: Timeline): TrackGroup => timeline.track_groups?.find(g => g.all_tracks) ?? {
  id: '', name: 'All Group', members: [], all_tracks: true, active: false, linked: [...GROUP_CONTROLS]
};
export const groupTracks = (timeline: Timeline, group: TrackGroup) => timeline.tracks.filter(t => t.kind === 'audio' && (group.all_tracks || group.members.includes(t.id))).map(t => t.id);
export function groupPatch(timeline: Timeline, group: TrackGroup, patch: Partial<TrackGroup>): Op {
  const { id, ...fields } = patch;
  return group.id ? {
    op: 'set_track_group', timeline: timeline.id, id: group.id, patch: fields
  } : { op: 'add_track_group', timeline: timeline.id, group: { ...group, ...fields } };
}
export function groupDefaults(): GroupControl[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(GROUP_DEFAULTS_KEY) ?? 'null');
    if (Array.isArray(value) && value.every(v => GROUP_CONTROLS.includes(v)))
      return [...new Set(value)] as GroupControl[];
  }
  catch { /* An unavailable user preference leaves the documented defaults. */
  }
  return [...GROUP_CONTROLS];
}
export function groupKey(timeline: Timeline, event: Pick<KeyboardEvent, 'ctrlKey' | 'shiftKey' | 'altKey' | 'metaKey' | 'code'>): Op[] {
  if (!event.ctrlKey || event.shiftKey || event.altKey || event.metaKey)
    return [];
  if (event.code === 'Numpad0')
    return [{ op: 'set_timeline_audio', timeline: timeline.id, patch: { groups_bypassed: !timeline.audio?.groups_bypassed } }];
  const group = event.code === 'NumpadMultiply' ? allGroup(timeline) : /^Numpad[1-9]$/.test(event.code) ? timeline.track_groups?.filter(g => !g.all_tracks)[Number(event.code.slice(-1)) - 1] : undefined;
  return group ? [groupPatch(timeline, group, { active: group.active === false })] : [];
}
