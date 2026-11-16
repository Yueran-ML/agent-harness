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
import { isPointSetting, keyOps, numbered, paramOf, pathOf, settingOf, settingsOf, wholeParam, type FusionSetting } from './settings';

type SettingLabels = Messages['label']['studio']['fusionPage']['settings'];

/** The name of each setting that takes keys, by its path in its node's `params`, as the Inspector writes it; but those of the points of a Polygon (`settingLabel`). */
export function settingNames(labels: SettingLabels): Record<string, string> {
  return {
    subtractive_additive: labels.subtractiveAdditive, alpha_gain: labels.alphaGain, burn_in: labels.burnIn, blend: labels.blend,
    'center.x': labels.centerX, 'center.y': labels.centerY, size: labels.size, angle: labels.angle,
    'color.r': labels.red, 'color.g': labels.green, 'color.b': labels.blue, 'color.a': labels.alpha,
    level: labels.level, soft_edge: labels.softEdge, border_width: labels.borderWidth,
    width: labels.width, height: labels.height, corner_radius: labels.cornerRadius,
    // A Blur, whose two sizes are named apart (but see `settingLabel`), and a Mosaic.
    'size.x': labels.blurSizeX, 'size.y': labels.blurSizeY,
    frequency: labels.pixelFrequency, aspect: labels.aspect, 'offset.x': labels.offsetX, 'offset.y': labels.offsetY,
  };
}

/** Whether a Blur has its X and Y locked: its `size.x` is its one Blur Size then, and its `size.y` is kept and not read. */
const lockedBlur = (node: FusionNode | undefined): boolean => node !== undefined && node.kind === 'blur' && node.params.lock_xy === true;

/** Whether a node keeps a setting and does not read it: the Blur Size down of a Blur whose X and Y are locked. */
const unread = (node: FusionNode, setting: FusionSetting): boolean => setting.path === 'size.y' && lockedBlur(node);

/**
 * The name of a setting by its path, as the Inspector writes it: a number of a point of a Polygon
 * with the number of its point, from 1 (`points.2.x` is Point 3 X); the `size.x` of a Blur (`node`)
 * whose X and Y are locked, which is its one Blur Size; a path that has no name, as it is.
 */
export function settingLabel(labels: SettingLabels, path: string, node?: FusionNode): string {
  const point = /^points\.(\d+)\.(x|y)$/.exec(path);
  if (point) return numbered(point[2] === 'x' ? labels.pointX : labels.pointY, Number(point[1]));
  if (path === 'size.x' && lockedBlur(node)) return labels.blurSize;
  return settingNames(labels)[path] ?? path;
}

/**
 * How the values of a setting are written beside the graph: an angle in degrees, a Blur Size to a
 * tenth as the Inspector writes it, an offset of a Mosaic in pixels, every other setting as the
 * number that it is.
 */
function displayOf(setting: FusionSetting): CurveDisplay {
  if (setting.unit === 'degrees_counterclockwise') return { factor: 1, unit: '°', digits: 1 };
  if (setting.unit === 'pixels1080') return { factor: 1, unit: '', digits: 1 };
  if (setting.unit === 'pixels') return { factor: 1, unit: 'px', digits: 1 };
  return { factor: 1, unit: '', digits: 3 };
}

/** A setting of a node that the editor draws, with the node and the setting that its curve is of. */
interface Drawn { curve: Curve; node: FusionNode; setting: FusionSetting }

/** Whether the engine refused a change because it changes nothing: a key dragged back to where it was, which is no failure. */
const isEmpty = (error: unknown): boolean => error instanceof EngineError && typeof error.data === 'object' && error.data !== null
  && (error.data as { reason?: unknown }).reason === 'empty';

/**
 * The Spline Editor of the Fusion page (DaVinci Resolve Reference Manual 20, Ch. 72, pp. 1476–1507):
 * the Curve Editor of the Edit page (`CurveEditor`) on the settings of the nodes of the current
 * clip's composition, over the frames that their keys count. With one node chosen it lists every
 * setting of that node that takes keys, those with keys first (of the points of a Polygon, which
 * may be many, only the numbers that have keys: a key is put on one in the Inspector; and the Blur
 * Size down of a Blur whose X and Y are locked, which the node does not read, only where it has
 * keys, its Blur Size across being named Blur Size as the Inspector names it); with none or
 * several chosen, the settings of every node that have keys, each named with its node.
 *
 * A drag of keys or of a handle is one gesture: each move sets the setting whole with
 * `set_fusion_keys`, the last move ends the gesture, and the whole is one step to undo; taken back,
 * the setting is set to what it was. Every other edit is one transaction of `splice_fusion_keys`,
 * of the keys that differ from those that the clip has as it goes, or of `set_fusion_keys` where
 * the last key of a setting is taken away (`keyOps`). A press on the ruler moves the playhead to
 * the frame of the timeline at which the clip shows that picture.
 */
export function FusionCurves({ bridge, session, revision, timeline, clip, media, composition, node, names, frame, onSeek, onClose, locked = false }: {
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
  /** Whether the clip lies on a locked track: its curves are shown, and none is changed, as those of a clip of the Edit page. */
  locked?: boolean;
}) {
  const words = useWords();
  const labels = words.label.studio.fusionPage;
  const [selection, setSelection] = useState<CurveSelection>(NO_SELECTION);
  const { commit, refusal, setRefusal } = useCommit({ bridge, revision, session, timeline });
  const gesture = useAudioGesture({
    bridge, session, revision, identity: `${timeline.id}/${clip.id}/fusion-curves`, locked,
    onFailure: (error) => { if (!isEmpty(error)) setRefusal(refusalOf(error, words.hint.studio)); },
  });
  /** Whether a drag has begun a gesture that is still to be ended or taken back. */
  const dragging = useRef(false);
  const at: FusionAt = { timeline: timeline.id, clip: clip.id };

  const settingName = (owner: FusionNode, path: string) => settingLabel(labels.settings, path, owner);
  // One node chosen: all that it has, but of the points of a Polygon, which may be many, and of what the node does not read (the
  // Blur Size down of a Blur whose X and Y are locked), those that have keys. Else what has keys, of every node.
  const of = node !== null ? [node] : composition.nodes;
  const drawn: Drawn[] = of.flatMap(each => settingsOf(each)
    .filter(setting => (node !== null && !isPointSetting(setting) && !unread(each, setting)) || typeof paramOf(each, setting) === 'object')
    .map(setting => ({ node: each, setting })))
    .map(({ node: each, setting }, index): Drawn => ({
      node: each, setting,
      curve: {
        id: pathOf(each, setting), color: CURVE_COLORS[index % CURVE_COLORS.length], param: paramOf(each, setting), bounds: setting.bounds, display: displayOf(setting),
        label: node !== null ? settingName(each, setting.path) : `${names.get(each.id) ?? each.id} · ${settingName(each, setting.path)}`,
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
    if (locked || wanted.length === 0) return;
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
    if (locked || wanted.length === 0) return;
    void commit((current) => {
      const located = clipLocations(current).get(clip.id);
      if (!located || located.track.locked) return [];
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
    {locked && <p className="caption" data-test="fusion-curves-locked">{words.hint.studio.inspector.locked}</p>}
    <CurveEditor curves={drawn.map(each => each.curve)} playhead={keyFrameOf(clip, timeline, media, frame)} range={shownKeyRange(clip, clock)}
      selection={selection} onSelect={setSelection} disabled={locked} title={labels.splineEditor} tools={close}
      onPreview={preview} onCommit={commitChanges} onCancel={cancel} onSeek={keyFrame => onSeek(frameOfKey(clip, timeline, media, keyFrame), false)} />
  </section>;
}
