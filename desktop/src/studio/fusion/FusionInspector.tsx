import { Fragment, useId, useRef, useState, type ReactNode } from 'react';
import { ChevronLeft, ChevronRight, Minus, Plus, X } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { COMPOSITES } from '../color/nodeEdits';
import {
  EngineError, type Clip, type Composite, type FusionFillMethod, type FusionMaskCommon, type FusionNode, type FusionPaintMode, type Media, type Op, type Param,
  type Timeline,
} from '../engine';
import { useFairlightText } from '../fairlight/words';
import { keyAt, keyFrameOf } from '../inspector/clipKeys';
import { ChoiceField, ColorField, KeyDiamond, NumberField, TextField } from '../inspector/fields';
import { refusalOf } from '../inspector/refusal';
import { useCommit, type Build } from '../inspector/useCommit';
import { valueAt } from '../keyframes/value';
import { clipLocations } from '../timeline/selection';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { insertPoint, LEAST_POINTS, MOST_POINTS, relabel, removePoint, shownComposition, withFirst, type FusionAt } from './edits';
import { isMaskKind } from './graph';
import {
  addKey, isPointSetting, keyTimesOf, numbered, paramOf, pointPath, putValue, removeKey, settingOf, wholeParam, wholeValue, type FusionSetting,
} from './settings';

/** How a setting is written and dragged: its decimals, how much a pixel of dragging changes it, its unit, and the range of its slider, where it has one. */
interface Shape { digits: number; scrub: number; unit?: string; slider?: { min: number; max: number; step: number } }

/** The ways that a mask is combined with the mask that comes into it, and the ways that a Polygon is filled, as the engine names them. */
const PAINT_MODES: readonly FusionPaintMode[] = ['merge', 'add', 'subtract'];
const FILLS: readonly FusionFillMethod[] = ['alternate', 'non_zero_winding'];

/**
 * How each kind of setting is shown, by its unit in the catalog. A place is a share of the picture,
 * with a slider over the picture itself (the field takes what lies outside it); a length that may
 * be less than nothing (the border of a mask) has a slider from one width of the picture the one
 * way to one the other; a number of a point of a Polygon has no slider, so that a list of points
 * stays short; an angle is in degrees, with a slider of one turn; a size has a slider up to five
 * times; every other setting is a number from 0 to 1.
 */
function shapeOf(setting: FusionSetting): Shape {
  if (isPointSetting(setting)) return { digits: 3, scrub: 0.002 };
  if (setting.unit === 'widths' || setting.unit === 'heights') {
    const least = setting.bounds.low < 0 && !setting.path.startsWith('center.') ? -1 : 0;
    return { digits: 3, scrub: 0.002, slider: { min: least, max: 1, step: 0.001 } };
  }
  if (setting.unit === 'degrees_counterclockwise') return { digits: 1, scrub: 0.5, unit: '°', slider: { min: -180, max: 180, step: 0.1 } };
  if (setting.path === 'size') return { digits: 3, scrub: 0.005, slider: { min: 0, max: 5, step: 0.01 } };
  return { digits: 3, scrub: 0.005, slider: { min: setting.bounds.low, max: setting.bounds.high, step: 0.001 } };
}

/** A share from 0 to 1 as two digits of a colour written `#rrggbb`. */
const hex = (share: number) => Math.round(Math.min(1, Math.max(0, share)) * 255).toString(16).padStart(2, '0');

/** Whether the engine refused a change because it changes nothing: a slider let go where it was, which is no failure. */
const isEmpty = (error: unknown): boolean => error instanceof EngineError && typeof error.data === 'object' && error.data !== null
  && (error.data as { reason?: unknown }).reason === 'empty';

/**
 * The Inspector of the Fusion page (DaVinci Resolve Reference Manual 20, Ch. 70, pp. 1432–1457):
 * the settings of the one node that is chosen, from the engine's catalog (`FUSION_SETTINGS`), with
 * their ranges and defaults:
 *
 * - a Merge: Apply Mode (the composite modes of the Edit page), Operator (Over alone, so it is shown
 *   and cannot be changed), Subtractive - Additive, Alpha Gain, Burn In, Blend, and where its
 *   foreground lies, how large it is and how far it is turned;
 * - a Transform: Center, Size, Angle and the two flips;
 * - a Background: its colour, as one colour and as its four channels;
 * - a mask (Reference Manual 20, Ch. 108): Level, Soft Edge, Border Width, Paint Mode, Invert and
 *   Solid, which every mask has; then its shape: of a Rectangle its Center, Width, Height, Corner
 *   Radius and Angle; of an Ellipse the same without the Corner Radius; of a Polygon its Center,
 *   Size, Angle and Fill Method, and the list of its points, each with its X and its Y from the
 *   centre. A point is chosen by its name; Add Point puts one after the point chosen, or after the
 *   last, and Remove Point takes the point chosen away, or the last (`insertPoint`, `removePoint`:
 *   the list is set whole, and the points that stay keep their keys). A polygon takes no more than
 *   128 points, and the page takes none away below three, which are the fewest that make a shape.
 *   Lengths are written as the engine counts them, in widths and heights of the picture;
 * - a MediaIn and a MediaOut: nothing but the name.
 *
 * Every number has the three buttons of keys beside it (to the key before, a key put or taken away
 * at the playhead, to the key after), as the Inspector of the Edit page has them. A setting that has
 * keys takes a value typed or dragged as a key at the playhead; one without is one number. A label
 * double-clicked puts the default back. A slider or a label dragged is one gesture: each move sets
 * the setting whole, the release ends it, and the whole is one step to undo. Every other change is
 * one transaction, worked out from the clip as the engine has it when it is sent, with the first
 * composition before it where the clip has none yet (`withFirst`). What the engine refuses is said
 * at the top; but not a change that changes nothing (`empty`), which is no failure.
 */
export function FusionInspector({ bridge, session, revision, timeline, clip, media, node, name, frame, onSeek }: {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number; timeline: Timeline;
  /** The clip whose composition the node is of. */
  clip: Clip;
  media: readonly Media[];
  /** The one node that is chosen, or null where none or several are. */
  node: FusionNode | null;
  /** The name that the node editor shows the node by. */
  name: string;
  /** The playhead, in frames of the timeline. */
  frame: number;
  onSeek: (frame: number, scrubbing: boolean) => void;
}) {
  const words = useWords();
  const t = useFairlightText();
  const labels = words.label.studio.fusionPage;
  const said = words.hint.studio.fusionPage;
  const flipId = useId();
  const at: FusionAt = { timeline: timeline.id, clip: clip.id };
  const has = clip.fusion !== undefined;
  const { commit, refusal, setRefusal } = useCommit({ bridge, revision, session, timeline });
  const failed = (error: unknown) => { if (!isEmpty(error)) setRefusal(refusalOf(error, words.hint.studio)); };
  const gesture = useAudioGesture({ bridge, session, revision, identity: `${timeline.id}/${clip.id}/fusion/${node?.id ?? ''}`, locked: false, onFailure: failed });
  /** The setting that is dragged, as it was when the drag began, and the frame of its keys that the drag is at. */
  const dragged = useRef<{ path: string; initial: Param; keyFrame: number } | null>(null);
  const now = useRef({ has });
  now.current = { has };
  /** The point of a Polygon that is chosen in the list of its points, from 0, or null: where Add Point puts one after, and what Remove Point takes away. */
  const [chosenPoint, setChosenPoint] = useState<number | null>(null);

  const problem = refusal && refusal.reason !== 'empty' && <div className="inspector-problem" role="alert" data-test="fusion-inspector-problem">
    <p>{refusal.text}</p>
    <button type="button" className="icon-btn" aria-label={words.label.close} onClick={() => setRefusal(null)}><X size={14} /></button>
  </div>;
  if (node === null) return <div className="inspector" data-test="fusion-inspector-body">{problem}<p className="caption" data-test="fusion-no-node">{said.chooseNode}</p></div>;

  // The node as the functions below read it: it is one from here on, which they are not told by the test above.
  const shown: FusionNode = node;
  const keyFrame = keyFrameOf(clip, timeline, media, frame);
  const id = shown.id;
  /** One transaction about the node, worked out from the clip and the node as they are when it is sent. */
  const build = (ops: (found: FusionNode, keyFrameNow: number, where: FusionAt) => Op[]): Build => (current) => {
    const located = clipLocations(current).get(clip.id);
    const found = located ? shownComposition(located.clip).nodes.find(each => each.id === id) : undefined;
    if (!located || !found) return [];
    const where = { timeline: current.id, clip: clip.id };
    return withFirst(where, located.clip.fusion !== undefined, ops(found, keyFrameOf(located.clip, current, media, frame), where));
  };
  /** The three buttons of a setting: to the key before, a key put or taken away at the playhead, to the key after. */
  const keys = (setting: FusionSetting, label: string) => {
    const param = paramOf(shown, setting);
    const hit = keyAt(param, keyFrame) !== null;
    const times = keyTimesOf(clip, timeline, media, param);
    const go = (direction: -1 | 1) => {
      const target = direction < 0 ? times.filter(time => time < frame).at(-1) : times.find(time => time > frame);
      if (target !== undefined) onSeek(target, false);
    };
    return <div className="audio-param-keys" role="group" aria-label={label} data-keys={setting.path}>
      <button type="button" className="icon-btn" aria-label={`${label}: ${t('Previous keyframe')}`} title={t('Previous keyframe')}
        disabled={!times.some(time => time < frame)} onClick={() => go(-1)}><ChevronLeft size={14} aria-hidden="true" /></button>
      <button type="button" className="icon-btn" aria-label={`${label}: ${t(hit ? 'Remove keyframe' : 'Add keyframe')}`} title={t(hit ? 'Remove keyframe' : 'Add keyframe')}
        aria-pressed={hit} data-keyed={typeof param === 'object' || undefined} data-hit={hit || undefined}
        onClick={() => void commit(build((found, keyFrameNow, where) => {
          const there = settingOf(found, setting.path);
          if (!there) return [];
          return keyAt(paramOf(found, there), keyFrameNow) ? removeKey(where, found, there, keyFrameNow) : addKey(where, found, there, keyFrameNow);
        }))}><KeyDiamond /></button>
      <button type="button" className="icon-btn" aria-label={`${label}: ${t('Next keyframe')}`} title={t('Next keyframe')}
        disabled={!times.some(time => time > frame)} onClick={() => go(1)}><ChevronRight size={14} aria-hidden="true" /></button>
    </div>;
  };

  /** A number of the node with its slider and its buttons of keys. */
  const number = (path: string, label: string): ReactNode => {
    const setting = settingOf(shown, path);
    if (!setting) return null;
    const shape = shapeOf(setting);
    const param = paramOf(shown, setting);
    return <Fragment key={path}>
      <NumberField name={path} label={label} value={valueAt(param, keyFrame)} fallback={setting.bounds.neutral} min={setting.bounds.low} max={setting.bounds.high}
        digits={shape.digits} scrub={shape.scrub} unit={shape.unit} slider={shape.slider}
        gesture={{
          start: () => {
            // What the drag puts back where it is taken back: the setting as it is now, or no composition where the clip has none.
            const began = gesture.start(now.current.has ? [wholeParam(at, shown, setting, param)] : [{ op: 'set_fusion', ...at, value: null }]);
            dragged.current = began ? { path, initial: param, keyFrame } : null;
            return began;
          },
          preview: (value) => {
            const drag = dragged.current;
            if (drag?.path === path) gesture.preview(withFirst(at, now.current.has, [wholeValue(at, shown, setting, drag.initial, value, drag.keyFrame)]));
          },
          end: () => { dragged.current = null; gesture.end(); },
          cancel: () => { dragged.current = null; gesture.cancel(); },
          active: () => gesture.active(),
        }}
        onCommit={value => void commit(build((found, keyFrameNow, where) => {
          const there = settingOf(found, path);
          return there ? putValue(where, found, there, value, keyFrameNow) : [];
        }))} />
      {keys(setting, label)}
    </Fragment>;
  };

  /** A switch of the node that takes no keys: a flip of a Transform, the Invert and the Solid of a mask. */
  const flag = (field: 'flip_horizontal' | 'flip_vertical' | 'invert' | 'solid', label: string, checked: boolean) => <div className="inspector-row" data-field={field}>
    <label htmlFor={`${flipId}-${field}`} className="inspector-label fusion-flag-label">{label}</label>
    <span className="inspector-value">
      <input id={`${flipId}-${field}`} type="checkbox" checked={checked}
        onChange={event => { const on = event.target.checked; void commit(build((found, _keyFrame, where) => [{ op: 'set_fusion_node', ...where, id: found.id, params: { [field]: on } }])); }} />
    </span>
  </div>;

  const settings = labels.settings;
  /** The controls that every mask has: its three numbers, how it is combined with the mask that comes into it, and its two switches. */
  const maskControls = (params: FusionMaskCommon): ReactNode => <>
    {number('level', settings.level)}
    {number('soft_edge', settings.softEdge)}
    {number('border_width', settings.borderWidth)}
    <ChoiceField<FusionPaintMode> name="paint_mode" label={settings.paintMode} value={params.paint_mode}
      options={PAINT_MODES.map(each => ({ value: each, label: labels.paintModes[each] }))}
      onCommit={mode => void commit(build((found, _keyFrame, where) => (isMaskKind(found.kind) && (found.params as FusionMaskCommon).paint_mode !== mode
        ? [{ op: 'set_fusion_node', ...where, id: found.id, params: { paint_mode: mode } }] : [])))} />
    {flag('invert', settings.invert, params.invert)}
    {flag('solid', settings.solid, params.solid)}
  </>;

  let body: ReactNode;
  if (node.kind === 'merge') {
    const apply = node.params.apply;
    body = <>
      <ChoiceField<Composite> name="apply" label={settings.apply} value={apply}
        options={COMPOSITES.map(each => ({ value: each, label: words.label.studio.colorPage.composites[each] }))}
        onCommit={mode => void commit(build((found, _keyFrame, where) => (found.kind === 'merge' && found.params.apply !== mode
          ? [{ op: 'set_fusion_node', ...where, id: found.id, params: { apply: mode } }] : [])))} />
      {/* Over is the one operator that the engine has: it is shown as Fusion shows it, and cannot be changed. */}
      <ChoiceField name="operator" label={settings.operator} value={node.params.operator} options={[{ value: 'over', label: settings.over }]} disabled onCommit={() => {}} />
      {number('subtractive_additive', settings.subtractiveAdditive)}
      {number('alpha_gain', settings.alphaGain)}
      {number('burn_in', settings.burnIn)}
      {number('blend', settings.blend)}
      <h3 className="inspector-section">{labels.inputs.foreground}</h3>
      {number('center.x', settings.centerX)}
      {number('center.y', settings.centerY)}
      {number('size', settings.size)}
      {number('angle', settings.angle)}
    </>;
  } else if (node.kind === 'transform') {
    body = <>
      {number('center.x', settings.centerX)}
      {number('center.y', settings.centerY)}
      {number('size', settings.size)}
      {number('angle', settings.angle)}
      {flag('flip_horizontal', settings.flipHorizontal, node.params.flip_horizontal)}
      {flag('flip_vertical', settings.flipVertical, node.params.flip_vertical)}
    </>;
  } else if (node.kind === 'background') {
    const channel = (path: string) => { const setting = settingOf(shown, path); return setting ? valueAt(paramOf(shown, setting), keyFrame) : 0; };
    const colour = `#${hex(channel('color.r'))}${hex(channel('color.g'))}${hex(channel('color.b'))}ff`;
    body = <>
      {/* The colour as one: its red, green and blue are set together, each where a value at the playhead goes; its alpha is the fourth number. */}
      <ColorField name="color" label={settings.color} value={colour} onCommit={(chosen) => {
        const shares = [1, 3, 5].map(from => Number((Number.parseInt(chosen.slice(from, from + 2), 16) / 255).toFixed(3)));
        void commit(build((found, keyFrameNow, where) => ['color.r', 'color.g', 'color.b'].flatMap((path, index) => {
          const there = settingOf(found, path);
          return there ? putValue(where, found, there, shares[index], keyFrameNow) : [];
        })));
      }} />
      {number('color.r', settings.red)}
      {number('color.g', settings.green)}
      {number('color.b', settings.blue)}
      {number('color.a', settings.alpha)}
    </>;
  } else if (node.kind === 'rectangle') {
    body = <>
      {maskControls(node.params)}
      {number('center.x', settings.centerX)}
      {number('center.y', settings.centerY)}
      {number('width', settings.width)}
      {number('height', settings.height)}
      {number('corner_radius', settings.cornerRadius)}
      {number('angle', settings.angle)}
    </>;
  } else if (node.kind === 'ellipse') {
    body = <>
      {maskControls(node.params)}
      {number('center.x', settings.centerX)}
      {number('center.y', settings.centerY)}
      {number('width', settings.width)}
      {number('height', settings.height)}
      {number('angle', settings.angle)}
    </>;
  } else if (node.kind === 'polygon') {
    const points = node.params.points;
    const count = points.length;
    // The point chosen, where the polygon still has it: one taken away by someone else is chosen no longer.
    const chosen = chosenPoint !== null && chosenPoint < count ? chosenPoint : null;
    body = <>
      {maskControls(node.params)}
      {number('center.x', settings.centerX)}
      {number('center.y', settings.centerY)}
      {number('size', settings.size)}
      {number('angle', settings.angle)}
      <ChoiceField<FusionFillMethod> name="fill" label={settings.fill} value={node.params.fill}
        options={FILLS.map(each => ({ value: each, label: labels.fills[each] }))}
        onCommit={fill => void commit(build((found, _keyFrame, where) => (found.kind === 'polygon' && found.params.fill !== fill
          ? [{ op: 'set_fusion_node', ...where, id: found.id, params: { fill } }] : [])))} />
      <h3 className="inspector-section">{settings.points}</h3>
      <div className="fusion-points-tools" role="group" aria-label={settings.points}>
        {/* After the point chosen, or after the last: the new point is the one chosen then, so that points are added one after the other. */}
        <button type="button" className="color-tool" disabled={count >= MOST_POINTS} data-test="add-point"
          onClick={() => {
            const after = chosen;
            void commit(build((found, keyFrameNow, where) => insertPoint(where, found, after, keyFrameNow))).then((sent) => {
              if (sent) setChosenPoint(after === null ? count : after + 1);
            });
          }}><Plus size={14} aria-hidden="true" />{settings.addPoint}</button>
        {/* The point chosen, or the last. The points that are left keep their numbers and their keys. */}
        <button type="button" className="color-tool" disabled={count <= LEAST_POINTS} data-test="remove-point"
          onClick={() => {
            const index = chosen ?? count - 1;
            setChosenPoint(null);
            void commit(build((found, _keyFrame, where) => (found.kind === 'polygon' && found.params.points.length > LEAST_POINTS ? removePoint(where, found, index) : [])));
          }}><Minus size={14} aria-hidden="true" />{settings.removePoint}</button>
      </div>
      {count < LEAST_POINTS && <p className="caption" data-test="fusion-few-points">{said.fewPoints}</p>}
      <ol className="fusion-points" data-test="fusion-points">
        {points.map((_, index) => <li key={index} className="fusion-point" data-point={index} data-chosen={chosen === index || undefined}>
          <button type="button" className="fusion-point-name" aria-pressed={chosen === index} onClick={() => setChosenPoint(chosen === index ? null : index)}>
            {numbered(settings.point, index)}</button>
          {number(pointPath(index, 'x'), numbered(settings.pointX, index))}
          {number(pointPath(index, 'y'), numbered(settings.pointY, index))}
        </li>)}
      </ol>
    </>;
  } else {
    body = <p className="caption" data-test="fusion-no-settings">{said.noSettings}</p>;
  }

  return <div className="inspector" data-test="fusion-inspector-body" data-node={node.id} data-kind={node.kind}>
    {problem}
    <div className="inspector-what">
      <span className="inspector-name" data-test="fusion-node-name">{name}</span>
      <span className="caption">{labels.kinds[node.kind]}</span>
    </div>
    <div className="inspector-panel" data-panel="fusion">
      <TextField name="label" label={labels.nodeName} value={node.label} allowEmpty
        onCommit={label => void commit(build((found, _keyFrame, where) => relabel(where, { nodes: [found], links: [] }, found.id, label.trim())))} />
      {body}
    </div>
  </div>;
}
