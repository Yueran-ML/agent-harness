/**
 * The clips of a timeline as the Color page of DaVinci Resolve lists them (Reference Manual 20,
 * pp. 2973–2974, 3024–3028): the clips of its video tracks, numbered by where their first frame
 * lies, left to right, whatever their track; the clip at the playhead; and what the badges of a
 * thumbnail say of each. Pure, so that the page and its tests work from the same rules.
 */
import type { Clip, ClipTrack, Grade, Group, Media, Rate, Timeline } from '../engine';
import { framesOf, rateOf } from '../timecode';

/** A clip of a video track, with its number on the Color page and the track it is on. */
export interface ColorClip {
  clip: Clip;
  /** 1 for the clip that begins first: the numbers stay as they are while clips are hidden or filtered. */
  number: number;
  track: ClipTrack;
  /** The track among the video tracks, from 1 for the lowest (V1). */
  trackNumber: number;
}

/** What the Clips menu of the Interface toolbar shows (p. 3031): all clips, or those of one kind. */
export type ClipFilter = 'all' | 'graded' | 'ungraded' | 'grouped' | 'matched';
export const CLIP_FILTERS: readonly ClipFilter[] = ['all', 'graded', 'ungraded', 'grouped', 'matched'];

/** The video tracks of a timeline, the lowest first (V1, V2 …). */
export function videoTracks(timeline: Timeline | null): ClipTrack[] {
  return (timeline?.tracks ?? []).filter((track): track is ClipTrack => track.kind === 'video');
}

/**
 * Every clip of the video tracks, numbered as the Color page numbers them (p. 3027): by the frame
 * that each begins at, and of two that begin together the one on the lower track first.
 */
export function numberedClips(timeline: Timeline | null): ColorClip[] {
  const all = videoTracks(timeline).flatMap((track, index) => track.clips.map(clip => ({ clip, track, trackNumber: index + 1 })));
  all.sort((a, b) => a.clip.start - b.clip.start || a.trackNumber - b.trackNumber);
  return all.map((each, index) => ({ ...each, number: index + 1 }));
}

/**
 * The clips that the thumbnail timeline shows: not those of a disabled track (p. 3025), nor those
 * of a track that the mini-timeline marks red, to be skipped (a state of the view, not of the project).
 */
export function shownClips(clips: readonly ColorClip[], skipped: ReadonlySet<string>): ColorClip[] {
  return clips.filter(each => each.track.enabled && !skipped.has(each.track.id));
}

/** Whether a grade colours anything: it has a node. A new clip's grade has none. */
export const hasNodes = (grade: Grade): boolean => grade.nodes.length > 0;

/** Whether a corrector of a grade has a shot match on it (p. 3048). */
export const isMatched = (grade: Grade): boolean => grade.nodes.some(node => node.kind === 'corrector' && node.params.shot_match != null);

/** What the thumbnail of a clip shows by its number and at its foot: graded, in a group, shot matched. */
export interface Badges { graded: boolean; grouped: boolean; matched: boolean }
export function badgesOf(clip: Clip): Badges {
  return { graded: hasNodes(clip.grade), grouped: clip.group !== null, matched: isMatched(clip.grade) };
}

/** Whether a clip is one that a filter of the Clips menu keeps. */
export function keptBy(filter: ClipFilter, clip: Clip): boolean {
  const badges = badgesOf(clip);
  switch (filter) {
    case 'graded': return badges.graded;
    case 'ungraded': return !badges.graded;
    case 'grouped': return badges.grouped;
    case 'matched': return badges.matched;
    default: return true;
  }
}

/** Whether `frame` of the timeline lies within a clip. */
export const covers = (clip: Clip, frame: number): boolean => clip.start <= frame && frame < clip.start + clip.length;

/**
 * The current clip, whose grade the node editor shows: the clip at the playhead (p. 3024). Where
 * clips lie on top of each other there, it is the one chosen last (a thumbnail clicked) if that one
 * is among them, else the one on the highest track, which is the one seen.
 */
export function currentClip(shown: readonly ColorClip[], frame: number | null, chosen: string | null): ColorClip | null {
  if (frame === null) return null;
  const here = shown.filter(each => covers(each.clip, frame));
  if (here.length === 0) return null;
  return here.find(each => each.clip.id === chosen) ?? here.reduce((top, each) => (each.trackNumber > top.trackNumber ? each : top));
}

/**
 * The clip that Up or Down goes to (p. 3024): the one before or after the current clip in the
 * thumbnail timeline. Where no clip is current (the playhead in a gap), the last that begins
 * before the playhead, or the first that begins after it.
 */
export function stepClip(shown: readonly ColorClip[], current: ColorClip | null, frame: number, direction: -1 | 1): ColorClip | null {
  const at = current === null ? -1 : shown.findIndex(each => each.clip.id === current.clip.id);
  if (at >= 0) return shown[at + direction] ?? null;
  if (direction > 0) return shown.find(each => each.clip.start > frame) ?? null;
  return [...shown].reverse().find(each => each.clip.start < frame) ?? null;
}

/**
 * The frame of its source that a clip shows `offset` frames after its first: the frame of its
 * media, at the media's own rate, through its speed and its direction. Titles and colours have none.
 */
export function sourceFrameAt(clip: Clip, offset: number, timelineRate: Rate, media: Media | undefined): number | null {
  if (clip.source.kind !== 'media' || media === undefined) return null;
  const rate = rateOf(media);
  const first = framesOf(clip.source.in, rate);
  const into = Math.max(0, Math.min(clip.length - 1, offset));
  const along = clip.reverse ? clip.length - 1 - into : into;
  // Seconds of the timeline, times the speed, in frames of the media.
  const frames = along * (timelineRate.den / timelineRate.num) * (clip.speed.num / clip.speed.den) * (rate.num / rate.den);
  return first + Math.floor(frames + 1e-6);
}

/** The grade of one of the four levels (p. 3330), for the current clip of a timeline. */
export type GradeLevel = 'pre_clip' | 'clip' | 'post_clip' | 'timeline';
/** The levels in the order a picture goes through them, as the node editor lists them. */
export const GRADE_LEVELS: readonly GradeLevel[] = ['pre_clip', 'clip', 'post_clip', 'timeline'];

/** The levels that the node editor offers: a clip in a group has its group's two besides its own (p. 3303). */
export function levelsOf(clip: Clip | null, groups: readonly Group[]): GradeLevel[] {
  const grouped = clip !== null && clip.group !== null && groups.some(group => group.id === clip.group);
  return grouped ? [...GRADE_LEVELS] : ['clip', 'timeline'];
}

/** The grade of a level, where there is one to show. */
export function gradeOf(level: GradeLevel, timeline: Timeline | null, clip: Clip | null, groups: readonly Group[]): Grade | null {
  if (level === 'timeline') return timeline?.grade ?? null;
  if (clip === null) return null;
  if (level === 'clip') return clip.grade;
  const group = groups.find(each => each.id === clip.group);
  if (!group) return null;
  return level === 'pre_clip' ? group.pre : group.post;
}
