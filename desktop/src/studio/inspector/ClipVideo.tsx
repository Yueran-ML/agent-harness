import { useWords } from '../../app/locale';
import { useContext } from 'react';
import { ChevronLeft, ChevronRight, Spline } from 'lucide-react';
import { COMPOSITES } from '../color/nodeEdits';
import { RETIME_PROCESSES, type Clip, type ClipKeyed, type Media, type Op, type Timeline } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { clipLocations } from '../timeline/selection';
import { addKey, keyAt, keyFrameOf, keyTimes, paramOf, putValue, removeKey, settingAt } from './clipKeys';
import { common, setClips, transitionEdges } from './common';
import { ChoiceField, FieldsLocked, KeyDiamond, NumberField } from './fields';
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
 * of the timeline, kept as a share of its size), rotation, the composite mode and opacity (in
 * percent, kept from 0 to 1), the cropping (in percent of the picture, kept as shares of it), the
 * Retime Process and the fades. Each change sets every clip chosen, in one transaction.
 *
 * The Retime Process (Retime and Scaling, Reference Manual 20, pp. 1081–1082) is what a clip shows at
 * a frame whose place in its source lies between two pictures: the nearest one, or the two blended.
 * It is of the clips of media whose picture lasts, which alone it is shown for and set on: a title,
 * a colour and a still show one picture however they are played.
 *
 * Each setting of the transform and of the crop takes keys (Reference Manual 20, Ch. 53). A field
 * shows what its setting is at the playhead; the diamond beside it puts a key there, or takes away
 * the one there, and the arrows go to the key before and after. A setting that has keys takes a
 * value typed or dragged as a key at the playhead; one without is one number, as before.
 *
 * Where the page has a Curve Editor (`onCurves`), a button at the top opens it on the clip chosen
 * (the Curve button of a keyframed clip in DaVinci Resolve, p. 1108).
 */
export function ClipVideo({ clips, timeline, commit, media = [], frame = 0, onSeek = () => {}, onCurves }: {
  clips: readonly Clip[]; timeline: Timeline; commit: Commit; media?: readonly Media[]; frame?: number;
  onSeek?: (frame: number, scrubbing: boolean) => void;
  onCurves?: () => void;
}) {
  const words = useWords().label.studio;
  const labels = words.inspector;
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
  const value = (path: ClipKeyed, shown: (kept: number) => number) => common(clips, clip => shown(settingAt(clip, path, at(clip))));
  const set = (path: ClipKeyed, kept: number) => void commit(each((clip, now) => putValue(now.id, clip, path, kept, at(clip, now))));
  const keys = (path: ClipKeyed, label: string) => {
    const hits = clips.map(clip => keyAt(paramOf(clip, path), at(clip)));
    const all = hits.length > 0 && hits.every(Boolean);
    const times = [...new Set(clips.flatMap(clip => keyTimes(clip, timeline, media, path)))].sort((a, b) => a - b);
    const go = (direction: -1 | 1) => {
      const target = direction < 0 ? times.filter(time => time < frame).at(-1) : times.find(time => time > frame);
      if (target !== undefined) onSeek(target, false);
    };
    // The diamond says three things apart: the setting has no keys, it has keys (`data-keyed`), a key is at the playhead (`data-hit`, and pressed where every clip has one).
    return <div className="audio-param-keys" role="group" aria-label={label} data-keys={path}>
      <button type="button" className="icon-btn" aria-label={label + ': ' + t('Previous keyframe')} title={t('Previous keyframe')} disabled={!times.some(time => time < frame)} onClick={() => go(-1)}><ChevronLeft size={14} aria-hidden="true" /></button>
      <button type="button" className="icon-btn" aria-label={label + ': ' + t(all ? 'Remove keyframe' : 'Add keyframe')} title={t(all ? 'Remove keyframe' : 'Add keyframe')} aria-pressed={all}
        data-keyed={times.length > 0 || undefined} data-hit={hits.some(Boolean) || undefined}
        disabled={locked} onClick={() => void commit(each((clip, now) => all ? removeKey(now.id, clip, path, at(clip, now)) : addKey(now.id, clip, path, at(clip, now))))}><KeyDiamond /></button>
      <button type="button" className="icon-btn" aria-label={label + ': ' + t('Next keyframe')} title={t('Next keyframe')} disabled={!times.some(time => time > frame)} onClick={() => go(1)}><ChevronRight size={14} aria-hidden="true" /></button>
    </div>;
  };
  /** A side of the crop or its softness, shown in percent of the picture. */
  const cropped = (name: string, path: ClipKeyed, label: string) => <>
    <NumberField name={name} label={label} value={value(path, kept => kept * 100)} fallback={0} unit="%"
      min={0} max={100} digits={1} scrub={0.25} slider={{ min: 0, max: 100, step: 0.1 }} onCommit={percent => set(path, percent / 100)} />
    {keys(path, label)}
  </>;
  const mode = common(clips, clip => clip.composite ?? 'normal');
  const lasts = (clip: Clip) => {
    if (clip.source.kind !== 'media') return false;
    const id = clip.source.media;
    return !media.find(item => item.id === id)?.facts.video?.still;
  };
  const retimed = clips.filter(lasts);
  const process = common(retimed, clip => clip.retime_process ?? 'nearest');
  return <div className="inspector-panel" data-panel="video">
    {onCurves && <button type="button" className="btn btn-quiet inspector-curves" onClick={onCurves} data-test="open-curves">
      <Spline size={14} aria-hidden="true" />{words.curves.title}
    </button>}
    <h3 className="inspector-section">{labels.transform}</h3>
    <NumberField name="zoom" label={labels.zoom} value={value('transform.scale', kept => kept)} fallback={1} min={0.01} max={16} digits={3} scrub={0.005}
      slider={{ min: 0.01, max: 4, step: 0.01 }} onCommit={scale => set('transform.scale', scale)} />
    {keys('transform.scale', labels.zoom)}
    <NumberField name="x" label={labels.positionX} value={value('transform.x', kept => kept * timeline.width)} fallback={0} unit="px"
      min={-timeline.width * 16} max={timeline.width * 16} digits={1} scrub={1} onCommit={x => set('transform.x', x / timeline.width)} />
    {keys('transform.x', labels.positionX)}
    <NumberField name="y" label={labels.positionY} value={value('transform.y', kept => kept * timeline.height)} fallback={0} unit="px"
      min={-timeline.height * 16} max={timeline.height * 16} digits={1} scrub={1} onCommit={y => set('transform.y', y / timeline.height)} />
    {keys('transform.y', labels.positionY)}
    <NumberField name="rotation" label={labels.rotation} value={value('transform.rotation', kept => kept)} fallback={0} unit="°"
      min={-3600} max={3600} digits={1} scrub={0.5} slider={{ min: -180, max: 180, step: 0.1 }} onCommit={rotation => set('transform.rotation', rotation)} />
    {keys('transform.rotation', labels.rotation)}
    <h3 className="inspector-section">{labels.composite}</h3>
    <ChoiceField name="composite" label={labels.compositeMode} value={mode} options={COMPOSITES.map(each => ({ value: each, label: words.colorPage.composites[each] }))}
      onCommit={composite => void commit(setClips(ids, () => ({ composite })))} />
    <NumberField name="opacity" label={labels.opacity} value={value('transform.opacity', kept => kept * 100)} fallback={100} unit="%"
      min={0} max={100} digits={1} scrub={0.5} slider={{ min: 0, max: 100, step: 0.1 }} onCommit={opacity => set('transform.opacity', opacity / 100)} />
    {keys('transform.opacity', labels.opacity)}
    <h3 className="inspector-section">{labels.cropping}</h3>
    {cropped('crop-left', 'crop.left', labels.cropLeft)}
    {cropped('crop-right', 'crop.right', labels.cropRight)}
    {cropped('crop-top', 'crop.top', labels.cropTop)}
    {cropped('crop-bottom', 'crop.bottom', labels.cropBottom)}
    {cropped('crop-softness', 'crop.softness', labels.softness)}
    {retimed.length > 0 && <>
      <h3 className="inspector-section">{labels.retimeAndScaling}</h3>
      <ChoiceField name="retime-process" label={labels.retimeProcess} value={process}
        options={RETIME_PROCESSES.map(each => ({ value: each, label: labels.retimeProcesses[each] }))}
        onCommit={chosen => void commit(setClips(retimed.map(clip => clip.id), clip => (clip.retime_process ?? 'nearest') === chosen ? null : { retime_process: chosen }))} />
    </>}
    <h3 className="inspector-section">{labels.fades}</h3>
    <Fades clips={clips} timeline={timeline} commit={commit} />
  </div>;
}
