import { Trash2 } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Cue, Timeline } from '../engine';
import { LONGEST_TEXT } from './ClipTitle';
import { NumberField, TextField } from './fields';
import type { Build, Commit } from './useCommit';

/** The longest a subtitle may be set to here, in frames: a day at 120 frames a second. */
const LONGEST_FRAMES = 120 * 60 * 60 * 24;

/** The subtitle `id` where it is now, with its track. */
export function cueOf(timeline: Timeline, id: string): Cue | null {
  for (const track of timeline.tracks) if (track.kind === 'subtitle') {
    const found = track.cues.find(cue => cue.id === id);
    if (found) return found;
  }
  return null;
}

/**
 * A subtitle, as DaVinci Resolve's Caption inspector: its text (Enter sets it, Shift+Enter starts a
 * new line), where it starts and how long it lasts, in frames of the timeline. Each change is one
 * transaction of `set_cue`, worked out from the subtitle as it is when sent; one that would run into
 * another subtitle is refused by the engine, and the inspector says so.
 */
export function CuePanel({ cue, commit }: { cue: Cue; commit: Commit }) {
  const labels = useWords().label.studio.inspector;
  const set = (change: Partial<Pick<Cue, 'start' | 'length' | 'text'>>) => {
    const build: Build = timeline => {
      const now = cueOf(timeline, cue.id);
      if (!now) return null;
      const changed = Object.fromEntries(Object.entries(change).filter(([key, value]) => now[key as keyof Cue] !== value));
      return Object.keys(changed).length === 0 ? null : [{ edit: 'set_cue', timeline: timeline.id, id: cue.id, ...changed }];
    };
    void commit(build);
  };
  return <div className="inspector-panel" data-panel="cue">
    <TextField name="cue_text" label={labels.text} value={cue.text} multiline
      onCommit={text => set({ text: [...text].slice(0, LONGEST_TEXT).join('') })} />
    <NumberField name="cue_start" label={labels.cueStart} value={cue.start} fallback={0} min={0} max={LONGEST_FRAMES} digits={0} scrub={0.25}
      onCommit={start => set({ start })} />
    <NumberField name="cue_length" label={labels.length} value={cue.length} fallback={cue.length} min={1} max={LONGEST_FRAMES} digits={0} scrub={0.25}
      onCommit={length => set({ length })} />
    <div className="inspector-actions">
      <button type="button" className="btn btn-quiet" data-test="remove-cue"
        onClick={() => void commit(timeline => (cueOf(timeline, cue.id) ? [{ edit: 'remove_cue', timeline: timeline.id, id: cue.id }] : null))}>
        <Trash2 size={14} aria-hidden="true" />{labels.removeCue}
      </button>
    </div>
  </div>;
}
