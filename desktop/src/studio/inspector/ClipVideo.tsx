import { useWords } from '../../app/locale';
import type { Clip, Timeline } from '../engine';
import { common, setClips, transitionEdges } from './common';
import { NumberField } from './fields';
import type { Commit } from './useCommit';

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
 */
export function ClipVideo({ clips, timeline, commit }: { clips: readonly Clip[]; timeline: Timeline; commit: Commit }) {
  const labels = useWords().label.studio.inspector;
  const ids = clips.map(clip => clip.id);
  const transform = (change: (clip: Clip) => Partial<Clip['transform']>) => void commit(setClips(ids, clip => ({ transform: { ...clip.transform, ...change(clip) } })));
  const value = (get: (clip: Clip) => number) => common(clips, get);
  return <div className="inspector-panel" data-panel="video">
    <h3 className="inspector-section">{labels.transform}</h3>
    <NumberField name="zoom" label={labels.zoom} value={value(clip => clip.transform.scale)} fallback={1} min={0.01} max={16} digits={3} scrub={0.005}
      slider={{ min: 0.01, max: 4, step: 0.01 }} onCommit={scale => transform(() => ({ scale }))} />
    <NumberField name="x" label={labels.positionX} value={value(clip => clip.transform.x * timeline.width)} fallback={0} unit="px"
      min={-timeline.width * 16} max={timeline.width * 16} digits={1} scrub={1} onCommit={x => transform(() => ({ x: x / timeline.width }))} />
    <NumberField name="y" label={labels.positionY} value={value(clip => clip.transform.y * timeline.height)} fallback={0} unit="px"
      min={-timeline.height * 16} max={timeline.height * 16} digits={1} scrub={1} onCommit={y => transform(() => ({ y: y / timeline.height }))} />
    <NumberField name="rotation" label={labels.rotation} value={value(clip => clip.transform.rotation)} fallback={0} unit="°"
      min={-3600} max={3600} digits={1} scrub={0.5} slider={{ min: -180, max: 180, step: 0.1 }} onCommit={rotation => transform(() => ({ rotation }))} />
    <h3 className="inspector-section">{labels.composite}</h3>
    <NumberField name="opacity" label={labels.opacity} value={value(clip => clip.transform.opacity * 100)} fallback={100} unit="%"
      min={0} max={100} digits={1} scrub={0.5} slider={{ min: 0, max: 100, step: 0.1 }} onCommit={opacity => transform(() => ({ opacity: opacity / 100 }))} />
    <h3 className="inspector-section">{labels.fades}</h3>
    <Fades clips={clips} timeline={timeline} commit={commit} />
  </div>;
}
