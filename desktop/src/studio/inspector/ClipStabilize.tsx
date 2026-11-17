import { useContext, useId } from 'react';
import { RotateCcw } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { analysedForAnother, DEFAULT_STABILIZE, STABILIZE_METHODS, type Clip, type Op, type Stabilize, type Timeline } from '../engine';
import { clipLocations } from '../timeline/selection';
import { common } from './common';
import { ChoiceField, FieldsLocked, NumberField } from './fields';
import type { Build, Commit } from './useCommit';
import { useStabilizing } from './useStabilizing';

/** The controls of a stabilizer that the Inspector sets one by one. */
type Control = Exclude<keyof Stabilize, 'analysis'>;

/** What a clip's stabilizer is, as the Inspector shows it: the one that a clip is given at first, where it has none. */
const shownOf = (clip: Clip): Stabilize => clip.stabilize ?? DEFAULT_STABILIZE;

/**
 * One transaction that sets the stabilizer of each of the clips that are still there as `change` makes it of the one it has
 * (null where it has none), each set whole, its analysis with it. A clip that it would not change is left out, as the engine
 * makes no step of one, and so is a clip on a locked track or on a track of sound, which has no stabilizer.
 */
export function setStabilizers(ids: readonly string[], change: (now: Stabilize | null) => Stabilize | null): Build {
  return timeline => {
    const locations = clipLocations(timeline);
    const ops: Op[] = [];
    for (const id of ids) {
      const located = locations.get(id);
      if (!located || located.track.locked || located.track.kind !== 'video') continue;
      const before = located.clip.stabilize ?? null;
      const after = change(before);
      if (JSON.stringify(before) !== JSON.stringify(after)) ops.push({ op: 'set_stabilize', timeline: timeline.id, clip: id, value: after });
    }
    return ops;
  };
}

/**
 * Stabilization of the Video tab (Reference Manual 20, pp. 633–634 and 1064–1065; PROTOCOL.md, "The stabilizer"): the Method,
 * Camera Lock, Zoom, the Cropping Ratio, Smooth and Strength of the stabilizer of the clips of moving pictures chosen, and the
 * Stabilize button that has the engine measure the motion of the picture of one of them.
 *
 * - A clip that has no stabilizer shows the one that a clip is given at first; the first change writes it whole, with the
 *   change, and Stabilize writes it whole with what was measured. Each change sets every clip chosen, in one transaction,
 *   as the other sections of the tab do; a control where they differ shows a dash.
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
  const value = <K extends Control>(key: K) => common(clips, clip => shownOf(clip)[key]);
  /** Set one control of every clip chosen: the stabilizer it has, or the one it is given at first, with the control changed. */
  const set = <K extends Control>(key: K, to: Stabilize[K]) => void commit(setStabilizers(ids, now => {
    const next: Stabilize = { ...(now ?? DEFAULT_STABILIZE) };
    next[key] = to;
    return next;
  }));
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
        onClick={() => void commit(setStabilizers(ids, () => null))}><RotateCcw size={12} aria-hidden="true" /></button>
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
