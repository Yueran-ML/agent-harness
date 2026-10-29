import { Trash2 } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Alignment, ClipTrack, Timeline, Transition, TransitionKind } from '../engine';
import type { Change } from '../timeline/commands';
import { ChoiceField, NumberField } from './fields';
import type { Commit } from './useCommit';

/** The kinds of transition that a track takes: three of picture, one of sound. */
export const KINDS: Record<'video' | 'audio', readonly TransitionKind[]> = { video: ['dissolve', 'dip_black', 'dip_white'], audio: ['crossfade'] };

/**
 * A transition, as DaVinci Resolve's Transition inspector: its kind, its duration in frames and
 * where it lies about the cut. The engine checks that both clips have the frames of their media that
 * it needs; where they have too few, it says the longest that fits, which can be taken at a click.
 */
export function TransitionPanel({ transition, track, timeline, commit }: {
  transition: Transition; track: ClipTrack; timeline: Timeline; commit: Commit;
}) {
  const words = useWords();
  const labels = words.label.studio.inspector;
  const names = words.label.studio.timeline;
  const set = (change: { kind?: TransitionKind; length?: number; align?: Alignment }) => {
    const build = (): Change[] => [{ edit: 'set_transition', timeline: timeline.id, id: transition.id, ...change }];
    void commit(build, { onLongest: length => void commit(() => [{ edit: 'set_transition', timeline: timeline.id, id: transition.id, length }]) });
  };
  const second = Math.max(1, Math.round(timeline.rate.num / timeline.rate.den));
  return <div className="inspector-panel" data-panel="transition">
    <ChoiceField name="kind" label={labels.kind} value={transition.kind} disabled={track.kind === 'audio'}
      options={KINDS[track.kind].map(kind => ({ value: kind, label: names[kind] }))} onCommit={kind => set({ kind })} />
    <NumberField name="length" label={labels.length} value={transition.length} fallback={second} min={1} max={1_000_000} digits={0} scrub={0.25}
      slider={{ min: 1, max: Math.max(second * 5, transition.length), step: 1 }} onCommit={length => set({ length })} />
    <ChoiceField name="align" label={labels.align} value={transition.align} onCommit={align => set({ align })} options={[
      { value: 'center', label: labels.middle }, { value: 'start', label: labels.start }, { value: 'end', label: labels.end },
    ]} />
    <div className="inspector-actions">
      <button type="button" className="btn btn-quiet" data-test="remove-transition"
        onClick={() => void commit(() => [{ edit: 'remove_transition', timeline: timeline.id, id: transition.id }])}>
        <Trash2 size={14} aria-hidden="true" />{labels.removeTransition}
      </button>
    </div>
  </div>;
}
