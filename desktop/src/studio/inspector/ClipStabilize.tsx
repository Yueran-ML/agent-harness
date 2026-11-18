import { useContext, useId } from 'react';
import { RotateCcw } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import {
  analysedForAnother, DEFAULT_STABILIZE, STABILIZE_CONTROLS, STABILIZE_METHODS,
  type Clip, type Op, type Stabilize, type StabilizeControl, type StabilizeSettings, type Timeline,
} from '../engine';
import { clipLocations } from '../timeline/selection';
import { common } from './common';
import { ChoiceField, FieldsLocked, NumberField } from './fields';
import type { Build, Commit } from './useCommit';
import { useStabilizing } from './useStabilizing';

/** What a clip's stabilizer is, as the Inspector shows it: the one that a clip is given at first, where it has none. */
const shownOf = (clip: Clip): Stabilize => clip.stabilize ?? DEFAULT_STABILIZE;

/** The clips of `ids` that are still there and may have a stabilizer: not on a locked track, nor on a track of sound. */
function stabilizable(timeline: Timeline, ids: readonly string[]): Clip[] {
  const locations = clipLocations(timeline);
  return ids.flatMap(id => {
    const located = locations.get(id);
    return located && !located.track.locked && located.track.kind === 'video' ? [located.clip] : [];
  });
}

/**
 * One transaction that sets the controls `settings` of the stabilizer of each of the clips that are still there, one
 * `set_stabilize_settings` of those controls alone for each: what was measured is kept by the engine and never sent back. A
 * clip that has no stabilizer is given the one that a clip has at first with them, as the engine gives it. A clip whose
 * stabilizer has them already is left out, as the engine makes no step of one, and so is a clip on a locked track or on a
 * track of sound, which has no stabilizer.
 */
export function setStabilizers(ids: readonly string[], settings: StabilizeSettings): Build {
  return timeline => {
    const ops: Op[] = [];
    for (const clip of stabilizable(timeline, ids)) {
      const now = clip.stabilize;
      const changes = now === undefined || STABILIZE_CONTROLS.some(key => settings[key] !== undefined && settings[key] !== now[key]);
      if (changes) ops.push({ op: 'set_stabilize_settings', timeline: timeline.id, clip: clip.id, settings: { ...settings } });
    }
    return ops;
  };
}

/** One transaction that takes the stabilizer away from each of the clips that has one, its analysis with it. */
export function resetStabilizers(ids: readonly string[]): Build {
  return timeline => stabilizable(timeline, ids).filter(clip => clip.stabilize !== undefined)
    .map((clip): Op => ({ op: 'set_stabilize', timeline: timeline.id, clip: clip.id, value: null }));
}

/**
 * Stabilization of the Video tab (Reference Manual 20, pp. 633–634 and 1064–1065; PROTOCOL.md, "The stabilizer"): the Method,
 * Camera Lock, Zoom, the Cropping Ratio, Smooth and Strength of the stabilizer of the clips of moving pictures chosen, and the
 * Stabilize button that has the engine measure the motion of the picture of one of them.
 *
 * - A clip that has no stabilizer shows the one that a clip is given at first; the first change gives it that one, with the
 *   change, and Stabilize has the engine write it whole with what was measured. Each change sends the one control changed
 *   (`set_stabilize_settings`), never what was measured, and sets every clip chosen, in one transaction, as the other
 *   sections of the tab do: a slider is sent once, as it is let go. A control where they differ shows a dash.
 * - The controls are numbers that take no keys. Camera Lock leaves Smooth and the Cropping Ratio no say, so they are off
 *   while it is on, as DaVinci's are.
 * - A change of the controls is drawn at once from the analysis that a clip has; a change of the Method needs a new one, and
 *   the analysis measured for the Method before is drawn until Stabilize is pressed again, which is said under the button.
 * - Stabilize measures one clip: with several chosen it is off and says so. While it measures, the section says how far it
 *   went and offers Stop, and its other controls are off, since a change of the project ends the measure.
 * - Reset takes the stabilizer away, the analysis with it.
 */
export function ClipStabilize({ clips, timeline, commit, bridge, session, revision }: {
  /** The clips of moving pictures chosen: of media that is not a still. */
  clips: readonly Clip[]; timeline: Timeline; commit: Commit;
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number; revision: number;
}) {
  const words = useWords();
  const labels = words.label.studio.inspector;
  const said = words.hint.studio.inspector.stabilize;
  const locked = useContext(FieldsLocked);
  const id = useId();
  const ids = clips.map(clip => clip.id);
  const one = clips.length === 1 ? clips[0] : null;
  const stabilizing = useStabilizing({ bridge, session, revision, timeline, clip: one, locked });
  const busy = stabilizing.running || stabilizing.asking;
  const value = <K extends StabilizeControl>(key: K) => common(clips, clip => shownOf(clip)[key]);
  /** Set one control of every clip chosen, that control alone: of the stabilizer it has, or of the one it is given at first. */
  const set = <K extends StabilizeControl>(key: K, to: Stabilize[K]) => {
    const settings: StabilizeSettings = {};
    settings[key] = to;
    void commit(setStabilizers(ids, settings));
  };
  const cameraLock = value('camera_lock');
  const zoom = value('zoom');
  const method = value('method');
  const status = stabilizing.status !== null && one !== null && stabilizing.status.clip === one.id ? stabilizing.status.text : null;
  const again = one !== null && analysedForAnother(one.stabilize) ? said.again.replace('{method}', () => labels.stabilizeMethods[one.stabilize!.method]) : null;
  /** A switch of the stabilizer: Camera Lock or Zoom. A mark where the clips differ. */
  const flag = (key: 'camera_lock' | 'zoom', name: string, label: string, checked: boolean | null) => <div className="inspector-row inspector-flag" data-field={name}
    data-mixed={checked === null || undefined}>
    <label htmlFor={`${id}-${name}`} className="inspector-label">{label}</label>
    <span className="inspector-value">
      <input id={`${id}-${name}`} type="checkbox" checked={checked === true} disabled={locked || busy}
        ref={node => { if (node) node.indeterminate = checked === null; }}
        onChange={event => set(key, event.target.checked)} />
    </span>
  </div>;
  const number = (key: 'cropping_ratio' | 'smooth' | 'strength', name: string, label: string, low: number, off: boolean) => <NumberField name={name} label={label}
    value={value(key)} fallback={DEFAULT_STABILIZE[key]} min={low} max={1} digits={3} scrub={0.005} slider={{ min: low, max: 1, step: 0.01 }}
    disabled={busy || off} onCommit={to => set(key, to)} />;

  return <div className="inspector-stabilize" data-section="stabilization">
    <div className="inspector-section-row">
      <h3 className="inspector-section">{labels.stabilization}</h3>
      <button type="button" className="icon-btn" aria-label={labels.resetStabilization} title={labels.resetStabilization} data-test="stabilize-reset"
        disabled={locked || busy || !clips.some(clip => clip.stabilize !== undefined)}
        onClick={() => void commit(resetStabilizers(ids))}><RotateCcw size={12} aria-hidden="true" /></button>
    </div>
    <ChoiceField name="stabilize-method" label={labels.stabilizeMethod} value={method} disabled={busy}
      options={STABILIZE_METHODS.map(each => ({ value: each, label: labels.stabilizeMethods[each] }))}
      onCommit={chosen => set('method', chosen)} />
    {flag('camera_lock', 'stabilize-camera-lock', labels.cameraLock, cameraLock)}
    {flag('zoom', 'stabilize-zoom', labels.zoom, zoom)}
    {number('cropping_ratio', 'stabilize-cropping-ratio', labels.croppingRatio, 0, cameraLock === true)}
    {number('smooth', 'stabilize-smooth', labels.smooth, 0, cameraLock === true)}
    {number('strength', 'stabilize-strength', labels.strength, -1, false)}
    <div className="stabilize-actions">
      <button type="button" className="btn btn-quiet" data-test="stabilize-run" disabled={locked || busy || one === null}
        onClick={stabilizing.start}>{labels.stabilize}</button>
      {stabilizing.running && <button type="button" className="btn btn-quiet" data-test="stabilize-stop" onClick={stabilizing.stop}>{labels.stopStabilizing}</button>}
    </div>
    {clips.length > 1 && <p className="caption stabilize-line" data-test="stabilize-one">{said.one}</p>}
    {status !== null && <p className="caption stabilize-line" role="status" data-test="stabilize-status">{status}</p>}
    {again !== null && <p className="caption stabilize-line stabilize-again" data-test="stabilize-again">{again}</p>}
  </div>;
}
