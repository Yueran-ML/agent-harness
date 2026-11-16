import { useRef, useState, type ReactNode } from 'react';
import { X } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { keyClockOf, shownKeyRange } from '../color/keyFrames';
import { EngineError, type Clip, type Composition, type FusionNode, type Media, type Op, type Timeline } from '../engine';
import { frameOfKey, keyFrameOf, mediaOf } from '../inspector/clipKeys';
import { refusalOf } from '../inspector/refusal';
import { useCommit } from '../inspector/useCommit';
import { CURVE_COLORS } from '../keyframes/ClipCurves';
import { CurveEditor, NO_SELECTION, type Curve, type CurveChange, type CurveDisplay, type CurveSelection } from '../keyframes/CurveEditor';
import type { Messages } from '../../messages/en';
import { clipLocations } from '../timeline/selection';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { shownComposition, type FusionAt } from './edits';
import { keyOps, paramOf, pathOf, settingOf, settingsOf, wholeParam, type FusionSetting } from './settings';

/** The name of each setting that takes keys, by its path in its node's `params`, as the Inspector writes it. */
export function settingNames(labels: Messages['label']['studio']['fusionPage']['settings']): Record<string, string> {
  return {
    subtractive_additive: labels.subtractiveAdditive, alpha_gain: labels.alphaGain, burn_in: labels.burnIn, blend: labels.blend,
    'center.x': labels.centerX, 'center.y': labels.centerY, size: labels.size, angle: labels.angle,
    'color.r': labels.red, 'color.g': labels.green, 'color.b': labels.blue, 'color.a': labels.alpha,
  };
}

/** How the values of a setting are written beside the graph: an angle in degrees, every other setting as the number that it is. */
const displayOf = (setting: FusionSetting): CurveDisplay => (setting.unit === 'degrees_counterclockwise' ? { factor: 1, unit: '°', digits: 1 } : { factor: 1, unit: '', digits: 3 });

/** A setting of a node that the editor draws, with the node and the setting that its curve is of. */
interface Drawn { curve: Curve; node: FusionNode; setting: FusionSetting }

/** Whether the engine refused a change because it changes nothing: a key dragged back to where it was, which is no failure. */
const isEmpty = (error: unknown): boolean => error instanceof EngineError && typeof error.data === 'object' && error.data !== null
  && (error.data as { reason?: unknown }).reason === 'empty';

/**
 * The Spline Editor of the Fusion page (DaVinci Resolve Reference Manual 20, Ch. 72, pp. 1476–1507):
 * the Curve Editor of the Edit page (`CurveEditor`) on the settings of the nodes of the current
 * clip's composition, over the frames that their keys count. With one node chosen it lists every
 * setting of that node that takes keys, those with keys first; with none or several chosen, the
 * settings of every node that have keys, each named with its node.
 *
 * A drag of keys or of a handle is one gesture: each move sets the setting whole with
 * `set_fusion_keys`, the last move ends the gesture, and the whole is one step to undo; taken back,
 * the setting is set to what it was. Every other edit is one transaction of `splice_fusion_keys`,
 * of the keys that differ from those that the clip has as it goes, or of `set_fusion_keys` where
 * the last key of a setting is taken away (`keyOps`). A press on the ruler moves the playhead to
 * the frame of the timeline at which the clip shows that picture.
 */
export function FusionCurves({ bridge, session, revision, timeline, clip, media, composition, node, names, frame, onSeek, onClose }: {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; timeline: Timeline;
  clip: Clip; media: readonly Media[];
  /** The composition as the page shows it. */
  composition: Composition;
  /** The one node that is chosen, or null where none or several are. */
  node: FusionNode | null;
  /** The name that the node editor shows each node by. */
  names: ReadonlyMap<string, string>;
  /** The playhead, in frames of the timeline. */
  frame: number;
  onSeek: (frame: number, scrubbing: boolean) => void;
  onClose: () => void;
}) {
  const words = useWords();
  const labels = words.label.studio.fusionPage;
  const [selection, setSelection] = useState<CurveSelection>(NO_SELECTION);
  const { commit, refusal, setRefusal } = useCommit({ bridge, revision, session, timeline });
  const gesture = useAudioGesture({
    bridge, session, revision, identity: `${timeline.id}/${clip.id}/fusion-curves`, locked: false,
    onFailure: (error) => { if (!isEmpty(error)) setRefusal(refusalOf(error, words.hint.studio)); },
  });
  /** Whether a drag has begun a gesture that is still to be ended or taken back. */
  const dragging = useRef(false);
  const at: FusionAt = { timeline: timeline.id, clip: clip.id };

  const settingName = settingNames(labels.settings);
  // One node chosen: all that it has. Else what has keys, of every node.
  const of = node !== null ? [node] : composition.nodes;
  const drawn: Drawn[] = of.flatMap(each => settingsOf(each)
    .filter(setting => node !== null || typeof paramOf(each, setting) === 'object')
    .map(setting => ({ node: each, setting })))
    .map(({ node: each, setting }, index): Drawn => ({
      node: each, setting,
      curve: {
        id: pathOf(each, setting), color: CURVE_COLORS[index % CURVE_COLORS.length], param: paramOf(each, setting), bounds: setting.bounds, display: displayOf(setting),
        label: node !== null ? settingName[setting.path] ?? setting.path : `${names.get(each.id) ?? each.id} · ${settingName[setting.path] ?? setting.path}`,
      },
    }));
  const byId = new Map(drawn.map(each => [each.curve.id, each]));

  const close: ReactNode = <button type="button" className="icon-btn" aria-label={labels.closeSpline} title={labels.closeSpline} onClick={onClose}
    data-test="close-spline"><X size={14} /></button>;
  const problem = refusal && <div className="inspector-problem" role="alert">
    <p>{refusal.text}</p>
    <button type="button" className="icon-btn" aria-label={words.label.close} onClick={() => setRefusal(null)}><X size={14} /></button>
  </div>;
  if (drawn.length === 0) {
    return <section className="curve-panel fusion-curves" aria-label={labels.splineEditor} data-test="fusion-curves">
      <header className="curve-editor-bar"><strong>{labels.splineEditor}</strong><span className="curve-editor-gap" />{close}</header>
      {problem}
      <p className="caption" data-test="fusion-curves-empty">{words.hint.studio.fusionPage.noCurves}</p>
    </section>;
  }

  const known = (changes: CurveChange[]) => changes.flatMap((change) => { const found = byId.get(change.id); return found ? [{ ...found, param: change.param }] : []; });
  const preview = (changes: CurveChange[]) => {
    const wanted = known(changes);
    if (wanted.length === 0) return;
    if (!dragging.current) {
      // What the gesture puts back where it is taken back: the settings as they are before its first move.
      if (!gesture.start(wanted.map(each => wholeParam(at, each.node, each.setting, each.curve.param)))) return;
      dragging.current = true;
    }
    gesture.preview(wanted.map((each): Op => ({ op: 'set_fusion_keys', ...at, path: each.curve.id, param: each.param })));
  };
  const commitChanges = (changes: CurveChange[]) => {
    // The end of a drag: its last move is what stays.
    if (dragging.current) { dragging.current = false; gesture.end(); return; }
    const wanted = known(changes);
    if (wanted.length === 0) return;
    void commit((current) => {
      const located = clipLocations(current).get(clip.id);
      if (!located) return [];
      const where = { timeline: current.id, clip: clip.id };
      return wanted.flatMap((each) => {
        // The node and the setting as the clip has them when the transaction goes.
        const found = shownComposition(located.clip).nodes.find(other => other.id === each.node.id);
        const setting = found ? settingOf(found, each.setting.path) : undefined;
        return found && setting ? keyOps(where, found, setting, each.param) : [];
      });
    });
  };
  const cancel = () => {
    if (!dragging.current) return;
    dragging.current = false;
    gesture.cancel();
  };

  const clock = keyClockOf(clip, timeline.rate, mediaOf(clip, media));
  return <section className="curve-panel fusion-curves" aria-label={labels.splineEditor} data-test="fusion-curves" data-clip={clip.id}>
    {problem}
    <CurveEditor curves={drawn.map(each => each.curve)} playhead={keyFrameOf(clip, timeline, media, frame)} range={shownKeyRange(clip, clock)}
      selection={selection} onSelect={setSelection} title={labels.splineEditor} tools={close}
      onPreview={preview} onCommit={commitChanges} onCancel={cancel} onSeek={keyFrame => onSeek(frameOfKey(clip, timeline, media, keyFrame), false)} />
  </section>;
}
