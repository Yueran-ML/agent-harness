import type { Clip, Rate, Timeline, Track } from '../engine';
import type { TimelineViewport } from '../timeline/Timeline';
import type { JumpTarget } from './viewState';

/** Frames of a time of sound, as the timeline counts them. */
const framesOf = (time: { ticks: number; scale: number }, rate: Rate) => time.ticks / time.scale * rate.num / rate.den;

/** Where a clip's fades of sound begin and end, in frames of the timeline. */
function fadePoints(clip: Clip, rate: Rate): number[] {
  const fadeIn = clip.audio_fade_in ? framesOf(clip.audio_fade_in.length, rate) : clip.fade_in;
  const fadeOut = clip.audio_fade_out ? framesOf(clip.audio_fade_out.length, rate) : clip.fade_out;
  const end = clip.start + clip.length;
  return [...(fadeIn > 0 ? [clip.start, clip.start + fadeIn] : []), ...(fadeOut > 0 ? [end - fadeOut, end] : [])];
}

const clipsOf = (tracks: readonly Track[]) => tracks.flatMap(track => track.kind === 'subtitle' ? [] : track.clips);

/**
 * The frame that Up or Down goes to when View Options › Jump to is Fade: the next or previous end
 * of a fade of sound (Reference Manual 20, pp. 3677–3678), or null where there is none that way.
 */
export function nextFade(timeline: Timeline, frame: number, direction: -1 | 1): number | null {
  const points = clipsOf(timeline.tracks.filter(track => track.kind === 'audio')).flatMap(clip => fadePoints(clip, timeline.rate))
    .map(point => Math.round(point));
  return nearest(points, frame, direction);
}

/**
 * Ctrl+Alt+Left and Right: the next or previous In or Out of a clip on the tracks chosen (p. 3748).
 * Timeline markers are not part of this project, so with no track chosen nothing moves.
 */
export function nextClipEdge(timeline: Timeline, tracks: readonly string[], frame: number, direction: -1 | 1): number | null {
  const chosen = timeline.tracks.filter(track => tracks.includes(track.id));
  const points = clipsOf(chosen).flatMap(clip => [clip.start, clip.start + clip.length]);
  return nearest(points, frame, direction);
}

function nearest(points: number[], frame: number, direction: -1 | 1): number | null {
  const ahead = points.filter(point => direction > 0 ? point > frame : point < frame);
  if (!ahead.length) return null;
  return direction > 0 ? Math.min(...ahead) : Math.max(...ahead);
}

/**
 * Smart Zoom (Ctrl+Alt+E, p. 3671): the tracks chosen share the height of the view between them,
 * and the first of them is scrolled to the top. `order` is the tracks as shown, `height` the height
 * of the view below the ruler. Tracks locked to a height keep it.
 */
export function smartZoom(viewport: TimelineViewport, order: readonly string[], chosen: readonly string[], locked: readonly string[],
  height: number, base: number): Pick<TimelineViewport, 'heights' | 'top'> | null {
  const zoomed = order.filter(id => chosen.includes(id));
  if (!zoomed.length) return null;
  const sized = zoomed.filter(id => !locked.includes(id));
  const fixed = zoomed.filter(id => locked.includes(id)).reduce((sum, id) => sum + (viewport.heights[id] ?? base), 0);
  const each = sized.length ? Math.max(24, Math.floor((height - fixed) / sized.length)) : 0;
  const heights = { ...viewport.heights, ...Object.fromEntries(sized.map(id => [id, each])) };
  let top = 0;
  for (const id of order) {
    if (id === zoomed[0]) break;
    top += heights[id] ?? base;
  }
  return { heights, top };
}

/** The choices of View Options › Jump to that this project offers: Marker and Transient are not made. */
export const JUMP_TARGETS: readonly JumpTarget[] = ['clip', 'fade'];
