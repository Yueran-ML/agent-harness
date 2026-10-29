import type { Alignment, Timeline, Transition } from '../engine';
import type { Material, Transaction } from '../timeline/commands';
import { KINDS } from '../inspector/TransitionPanel';
import { isTransition, type Effect } from './drag';

/** How near a cut, in pixels, a transition is dropped onto it; nearer than the second, it is centred on it. */
export const CUT_ZONE_PX = 12;
export const CENTER_ZONE_PX = 4;
/** How long a transition and a title or a solid colour are made, as DaVinci Resolve's standard durations: 1 and 5 seconds. */
export const TRANSITION_SECONDS = 1;
export const GENERATOR_SECONDS = 5;

export type EffectTarget =
  | { kind: 'cut'; track: string; left: string; right: string; cut: number; align: Alignment; existing: Transition | null }
  | { kind: 'place'; track: string; at: number; length: number };

const framesIn = (seconds: number, timeline: Timeline) => Math.max(1, Math.round(seconds * timeline.rate.num / timeline.rate.den));

/**
 * Where an effect dragged over a track goes, or null where it cannot be dropped there.
 *
 * A transition goes to the cut nearest `position` (in frames, not rounded) within a few pixels, on
 * a track of its kind: dropped just before the cut it ends on it, just after it starts on it, and
 * on it it is centred, as DaVinci Resolve aligns a transition dropped on an edit. A title or a solid
 * colour goes at `frame` on a video track where nothing is, for 5 seconds or until the next clip.
 */
export function effectTarget(timeline: Timeline, effect: Effect, trackId: string, position: number, frame: number, scale: number): EffectTarget | null {
  const track = timeline.tracks.find(each => each.id === trackId);
  if (!track || track.kind === 'subtitle' || track.locked) return null;
  if (isTransition(effect)) {
    if (!KINDS[track.kind].includes(effect)) return null;
    let best: { left: string; right: string; cut: number; offset: number } | null = null;
    for (let i = 1; i < track.clips.length; i += 1) {
      const left = track.clips[i - 1];
      const right = track.clips[i];
      if (left.start + left.length !== right.start) continue;
      const offset = (position - right.start) * scale;
      if (Math.abs(offset) > CUT_ZONE_PX || (best && Math.abs(best.offset) <= Math.abs(offset))) continue;
      best = { left: left.id, right: right.id, cut: right.start, offset };
    }
    if (!best) return null;
    const align: Alignment = Math.abs(best.offset) <= CENTER_ZONE_PX ? 'center' : best.offset < 0 ? 'end' : 'start';
    const existing = track.transitions.find(each => each.left === best.left && each.right === best.right) ?? null;
    return { kind: 'cut', track: track.id, left: best.left, right: best.right, cut: best.cut, align, existing };
  }
  if (track.kind !== 'video') return null;
  const at = Math.max(0, Math.round(frame));
  if (track.clips.some(clip => clip.start <= at && at < clip.start + clip.length)) return null;
  const next = Math.min(Infinity, ...track.clips.filter(clip => clip.start > at).map(clip => clip.start));
  const length = Math.min(framesIn(GENERATOR_SECONDS, timeline), next - at);
  return length >= 1 ? { kind: 'place', track: track.id, at, length } : null;
}

/**
 * The transaction of a drop: a transition added at the cut (or the kind of the one there changed,
 * as a transition dropped on another replaces it), or a title or a solid colour put on the track.
 */
export function effectTransaction(timeline: Timeline, revision: number, effect: Effect, target: EffectTarget, titleText: string): Transaction {
  if (target.kind === 'cut') {
    if (!isTransition(effect)) throw new Error(`${effect} is not a transition`);
    return {
      revision,
      ops: [target.existing
        ? { edit: 'set_transition', timeline: timeline.id, id: target.existing.id, kind: effect }
        : { edit: 'add_transition', timeline: timeline.id, left: target.left, right: target.right, kind: effect, length: framesIn(TRANSITION_SECONDS, timeline), align: target.align }],
    };
  }
  const source: Material = effect === 'title'
    ? { kind: 'title', text: titleText, length: target.length, style: { font: '', size: Math.max(1, Math.round(timeline.height * 0.065)), color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' } }
    : { kind: 'color', color: '#000000ff', length: target.length };
  return { revision, ops: [{ edit: 'overwrite', timeline: timeline.id, source, at: target.at, video_track: target.track, audio_track: null }] };
}
