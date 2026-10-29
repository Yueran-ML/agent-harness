import { useWords } from '../../app/locale';
import type { Clip, Timeline } from '../engine';
import { Fades } from './ClipVideo';
import { common, setClips } from './common';
import { NumberField } from './fields';
import type { Commit } from './useCommit';

/**
 * The sound of clips on audio tracks, as DaVinci Resolve's Audio tab: volume in decibels (from -96
 * to +24), pan from -100 (left) to 100 (right), kept from -1 to 1, and the fades.
 */
export function ClipAudio({ clips, timeline, commit }: { clips: readonly Clip[]; timeline: Timeline; commit: Commit }) {
  const labels = useWords().label.studio.inspector;
  const ids = clips.map(clip => clip.id);
  return <div className="inspector-panel" data-panel="audio">
    <NumberField name="volume" label={labels.volume} value={common(clips, clip => clip.gain_db)} fallback={0} unit="dB"
      min={-96} max={24} digits={1} scrub={0.1} slider={{ min: -96, max: 24, step: 0.1 }}
      onCommit={gain => void commit(setClips(ids, () => ({ gain_db: gain })))} />
    <NumberField name="pan" label={labels.pan} value={common(clips, clip => clip.pan * 100)} fallback={0}
      min={-100} max={100} digits={0} scrub={0.5} slider={{ min: -100, max: 100, step: 1 }}
      onCommit={pan => void commit(setClips(ids, () => ({ pan: pan / 100 })))} />
    <h3 className="inspector-section">{labels.fades}</h3>
    <Fades clips={clips} timeline={timeline} commit={commit} />
  </div>;
}
