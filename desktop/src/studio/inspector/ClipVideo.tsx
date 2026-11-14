import { useWords } from '../../app/locale';
import { useContext } from 'react';
import type { Clip, Media, Op, Timeline } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { clipLocations } from '../timeline/selection';
import { addKey, keyAt, keyFrameOf, keyTimes, putValue, removeKey, settingAt, type TransformSetting } from './clipKeys';
import { common, setClips, transitionEdges } from './common';
import { FieldsLocked, NumberField } from './fields';
import type { Build, Commit } from './useCommit';

/**
 * The fades of clips, in frames: each at most what the clip has beside its other fade. An edge with
 * a transition takes no fade, so the field is off where any of the clips has one there.
 */
export function Fades({ clips, timeline, commit, fineModifiers = false }: { clips: readonly Clip[]; timeline: Timeline; commit: Commit; fineModifiers?: boolean }) {
  const words = useWords();
  const labels = words.label.studio.inspector;
  const ids = clips.map(clip => clip.id);
  const edges = transitionEdges(timeline);
  const longest = Math.max(0, ...clips.map(clip => clip.length));
  const blocked = (edge: 'in' | 'out') => clips.some(clip => edges.get(clip.id)?.[edge]);
  return <>
    {(['in', 'out'] as const).map(edge => {
      const key = edge === 'in' ? 'fade_in' : 'fade_out';
      const other = edge === 'in' ? 'fade_out' : 'fade_in';
      const off = blocked(edge);
      return <NumberField key={edge} name={key} label={edge === 'in' ? labels.fadeIn : labels.fadeOut} value={common(clips, clip => clip[key])}
        fallback={0} min={0} max={longest} digits={0} scrub={0.5} slider={{ min: 0, max: Math.max(1, longest), step: 1 }}
        disabled={off} fineModifiers={fineModifiers} title={off ? words.hint.studio.inspector.fadeAtTransition : undefined}
        onCommit={value => void commit(setClips(ids, clip => ({ [key]: Math.max(0, Math.min(value, clip.length - clip[other])) })))} />;
    })}
  </>;
}

/**
 * The picture of clips on video tracks, as DaVinci Resolve's Video tab: zoom, position (in pixels
 * of the timeline, kept as a share of its size), rotation and opacity (in percent, kept from 0 to 1),
 * and the fades. Each change sets every clip chosen, in one transaction.
 *
 * Each setting of the transform takes keys (Reference Manual 20, Ch. 53). A field shows what its
 * setting is at the playhead; the diamond beside it puts a key there, or takes away the one there,
 * and the arrows go to the key before and after. A setting that has keys takes a value typed or
 * dragged as a key at the playhead; one without is one number, as before.
 */
export function ClipVideo({ clips, timeline, commit, media = [], frame = 0, onSeek = () => {} }: {
  clips: readonly Clip[]; timeline: Timeline; commit: Commit; media?: readonly Media[]; frame?: number;
  onSeek?: (frame: number, scrubbing: boolean) => void;
}) {
  const labels = useWords().label.studio.inspector;
  const t = useFairlightText();
  const locked = useContext(FieldsLocked);
  const ids = clips.map(clip => clip.id);
  const at = (clip: Clip, now: Pick<Timeline, 'rate'> = timeline) => keyFrameOf(clip, now, media, frame);
  /** What each chosen clip that may be set gives, worked out from the timeline as it is when it is sent. */
  const each = (ops: (clip: Clip, now: Timeline) => Op[]): Build => now => {
    const locations = clipLocations(now);
    return ids.flatMap(id => {
      const located = locations.get(id);
      return !located || located.track.locked ? [] : ops(located.clip, now);
    });
  };
  const value = (name: TransformSetting, shown: (kept: number) => number) => common(clips, clip => shown(settingAt(clip, name, at(clip))));
  const set = (name: TransformSetting, kept: (now: Timeline) => number) => void commit(each((clip, now) => putValue(now.id, clip, name, kept(now), at(clip, now))));
  const keys = (name: TransformSetting, label: string) => {
    const hits = clips.map(clip => keyAt(clip.transform[name], at(clip)));
    const all = hits.length > 0 && hits.every(Boolean);
    const times = [...new Set(clips.flatMap(clip => keyTimes(clip, timeline, media, name)))].sort((a, b) => a - b);
    const go = (direction: -1 | 1) => {
      const target = direction < 0 ? times.filter(time => time < frame).at(-1) : times.find(time => time > frame);
      if (target !== undefined) onSeek(target, false);
    };
    return <div className="audio-param-keys" role="group" aria-label={label} data-keys={name}>
      <button type="button" className="icon-btn" aria-label={label + ': ' + t('Previous keyframe')} disabled={!times.some(time => time < frame)} onClick={() => go(-1)}>‹</button>
      <button type="button" className="icon-btn" aria-label={label + ': ' + t(all ? 'Remove keyframe' : 'Add keyframe')} aria-pressed={all} data-hit={hits.some(Boolean) || undefined}
        disabled={locked} onClick={() => void commit(each((clip, now) => all ? removeKey(now.id, clip, name, at(clip, now)) : addKey(now.id, clip, name, at(clip, now))))}>◆</button>
      <button type="button" className="icon-btn" aria-label={label + ': ' + t('Next keyframe')} disabled={!times.some(time => time > frame)} onClick={() => go(1)}>›</button>
    </div>;
  };
  return <div className="inspector-panel" data-panel="video">
    <h3 className="inspector-section">{labels.transform}</h3>
    <NumberField name="zoom" label={labels.zoom} value={value('scale', kept => kept)} fallback={1} min={0.01} max={16} digits={3} scrub={0.005}
      slider={{ min: 0.01, max: 4, step: 0.01 }} onCommit={scale => set('scale', () => scale)} />
    {keys('scale', labels.zoom)}
    <NumberField name="x" label={labels.positionX} value={value('x', kept => kept * timeline.width)} fallback={0} unit="px"
      min={-timeline.width * 16} max={timeline.width * 16} digits={1} scrub={1} onCommit={x => set('x', () => x / timeline.width)} />
    {keys('x', labels.positionX)}
    <NumberField name="y" label={labels.positionY} value={value('y', kept => kept * timeline.height)} fallback={0} unit="px"
      min={-timeline.height * 16} max={timeline.height * 16} digits={1} scrub={1} onCommit={y => set('y', () => y / timeline.height)} />
    {keys('y', labels.positionY)}
    <NumberField name="rotation" label={labels.rotation} value={value('rotation', kept => kept)} fallback={0} unit="°"
      min={-3600} max={3600} digits={1} scrub={0.5} slider={{ min: -180, max: 180, step: 0.1 }} onCommit={rotation => set('rotation', () => rotation)} />
    {keys('rotation', labels.rotation)}
    <h3 className="inspector-section">{labels.composite}</h3>
    <NumberField name="opacity" label={labels.opacity} value={value('opacity', kept => kept * 100)} fallback={100} unit="%"
      min={0} max={100} digits={1} scrub={0.5} slider={{ min: 0, max: 100, step: 0.1 }} onCommit={opacity => set('opacity', () => opacity / 100)} />
    {keys('opacity', labels.opacity)}
    <h3 className="inspector-section">{labels.fades}</h3>
    <Fades clips={clips} timeline={timeline} commit={commit} />
  </div>;
}
