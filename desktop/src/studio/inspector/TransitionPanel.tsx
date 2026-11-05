import { Trash2 } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Alignment, ClipTrack, CrossfadeLevel, Timeline, Transition, TransitionKind } from '../engine';
import type { Change, Edit } from '../timeline/commands';
import { ChoiceField, FieldsLocked, NumberField } from './fields';
import { useFairlightText } from '../fairlight/words';
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
  const t = useFairlightText();
  const labels = words.label.studio.inspector;
  const names = words.label.studio.timeline;
  // A refusal can outlive a track lock. Recheck the accepted timeline when a queued edit or
  // the saved Use Longest callback builds its transaction, not only when a field was changed.
  const build = (change: Edit) => (current: Timeline): Change[] => current.id === timeline.id && current.tracks.some(each =>
    each.kind !== 'subtitle' && !each.locked && each.transitions.some(item => item.id === transition.id)) ? [change] : [];
  const set = (change: { kind?: TransitionKind; length?: number; align?: Alignment; fade_in_level_db?: CrossfadeLevel; fade_out_level_db?: CrossfadeLevel }) => {
    if (track.locked) return;
    void commit(build({ edit: 'set_transition', timeline: timeline.id, id: transition.id, ...change }), {
      onLongest: length => void commit(build({ edit: 'set_transition', timeline: timeline.id, id: transition.id, length })),
    });
  };
  const second = Math.max(1, Math.round(timeline.rate.num / timeline.rate.den));
  return <FieldsLocked.Provider value={track.locked}><div className="inspector-panel" data-panel="transition">
    <ChoiceField name="kind" label={labels.kind} value={transition.kind} disabled={track.kind === 'audio'}
      options={KINDS[track.kind].map(kind => ({ value: kind, label: names[kind] }))} onCommit={kind => set({ kind })} />
    <NumberField name="length" label={labels.length} value={transition.length} fallback={second} min={1} max={1_000_000} digits={0} scrub={0.25}
      slider={{ min: 1, max: Math.max(second * 5, transition.length), step: 1 }} onCommit={length => set({ length })} />
    <ChoiceField name="align" label={labels.align} value={transition.align} onCommit={align => set({ align })} options={[
      { value: 'center', label: labels.middle }, { value: 'start', label: labels.start }, { value: 'end', label: labels.end },
    ]} />
    {transition.kind === 'crossfade' && <>
      <ChoiceField name="fade-in-level" label={t('Fade In Level')} value={String(transition.fade_in_level_db ?? 0)}
        options={([-3, 0, 3] as const).map(level => ({ value: String(level), label: `${level > 0 ? '+' : ''}${level} dB` }))}
        onCommit={level => set({ fade_in_level_db: Number(level) as CrossfadeLevel })} />
      <ChoiceField name="fade-out-level" label={t('Fade Out Level')} value={String(transition.fade_out_level_db ?? 0)}
        options={([-3, 0, 3] as const).map(level => ({ value: String(level), label: `${level > 0 ? '+' : ''}${level} dB` }))}
        onCommit={level => set({ fade_out_level_db: Number(level) as CrossfadeLevel })} />
    </>}
    <div className="inspector-actions">
      <button type="button" className="btn btn-quiet" data-test="remove-transition" disabled={track.locked}
        onClick={() => void commit(build({ edit: 'remove_transition', timeline: timeline.id, id: transition.id }))}>
        <Trash2 size={14} aria-hidden="true" />{labels.removeTransition}
      </button>
    </div>
  </div></FieldsLocked.Provider>;
}
