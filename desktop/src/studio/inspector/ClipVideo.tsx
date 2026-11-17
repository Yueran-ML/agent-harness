import { useWords } from '../../app/locale';
import { Spline } from 'lucide-react';
import type { EngineBridge } from '../../bridge';
import { COMPOSITES } from '../color/nodeEdits';
import { RETIME_PROCESSES, type Clip, type ClipKeyed, type Media, type Timeline } from '../engine';
import { ClipStabilize } from './ClipStabilize';
import { useClipKeyFields } from './ClipKeyFields';
import { common, setClips, transitionEdges } from './common';
import { ChoiceField, NumberField } from './fields';
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
 * value typed or dragged as a key at the playhead; one without is one number, as before. The
 * fields and their buttons are those of `useClipKeyFields`, which the Title tab has too.
 *
 * Where the page has a Curve Editor (`onCurves`), a button at the top opens it on the clip chosen
 * (the Curve button of a keyframed clip in DaVinci Resolve, p. 1108).
 *
 * Where it is given the engine (`engine`), Stabilization comes after the cropping, as in DaVinci Resolve's Video tab, for
 * the clips of moving pictures, which alone have a motion to measure (`ClipStabilize`).
 */
export function ClipVideo({ clips, timeline, commit, media = [], frame = 0, onSeek = () => {}, onCurves, engine }: {
  clips: readonly Clip[]; timeline: Timeline; commit: Commit; media?: readonly Media[]; frame?: number;
  onSeek?: (frame: number, scrubbing: boolean) => void;
  onCurves?: () => void;
  /** The engine that the Stabilize button asks, the session it is of and the revision of the project. */
  engine?: { bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number; revision: number };
}) {
  const words = useWords().label.studio;
  const labels = words.inspector;
  const ids = clips.map(clip => clip.id);
  const { value, set, keys } = useClipKeyFields({ clips, timeline, commit, media, frame, onSeek });
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
    {engine && retimed.length > 0 && <ClipStabilize clips={retimed} timeline={timeline} commit={commit}
      bridge={engine.bridge} session={engine.session} revision={engine.revision} />}
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
