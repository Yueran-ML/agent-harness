import { Fragment, useId, useRef, type ReactNode } from 'react';
import { ChevronLeft, ChevronRight, X } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { COMPOSITES } from '../color/nodeEdits';
import { EngineError, type Clip, type Composite, type FusionNode, type Media, type Op, type Param, type Timeline } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { keyAt, keyFrameOf } from '../inspector/clipKeys';
import { ChoiceField, ColorField, KeyDiamond, NumberField, TextField } from '../inspector/fields';
import { refusalOf } from '../inspector/refusal';
import { useCommit, type Build } from '../inspector/useCommit';
import { valueAt } from '../keyframes/value';
import { clipLocations } from '../timeline/selection';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { relabel, shownComposition, withFirst, type FusionAt } from './edits';
import { addKey, keyTimesOf, paramOf, putValue, removeKey, settingOf, wholeParam, wholeValue, type FusionSetting } from './settings';

/** How a setting is written and dragged: its decimals, how much a pixel of dragging changes it, its unit, and the range of its slider. */
interface Shape { digits: number; scrub: number; unit?: string; slider: { min: number; max: number; step: number } }

/**
 * How each kind of setting is shown, by its unit in the catalog. A place is a share of the picture,
 * with a slider over the picture itself (the field takes what lies outside it); an angle is in
 * degrees, with a slider of one turn; a size has a slider up to five times; every other setting is
 * a number from 0 to 1.
 */
function shapeOf(setting: FusionSetting): Shape {
  if (setting.unit === 'widths' || setting.unit === 'heights') return { digits: 3, scrub: 0.002, slider: { min: 0, max: 1, step: 0.001 } };
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
 * - a MediaIn and a MediaOut: nothing but the name.
 *
 * Every number has the three buttons of keys beside it (to the key before, a key put or taken away
 * at the playhead, to the key after), as the Inspector of the Edit page has them. A setting that has
 * keys takes a value typed or dragged as a key at the playhead; one without is one number. A label
 * double-clicked puts the default back. A slider or a label dragged is one gesture: each move sets
 * the setting whole, the release ends it, and the whole is one step to undo. Every other change is
 * one transaction, worked out from the clip as the engine has it when it is sent, with the first
 * composition before it where the clip has none yet (`withFirst`). What the engine refuses is said
 * at the top.
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

  const problem = refusal && <div className="inspector-problem" role="alert" data-test="fusion-inspector-problem">
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

  /** A switch of the node that takes no keys: a flip of a Transform. */
  const flag = (field: 'flip_horizontal' | 'flip_vertical', label: string, checked: boolean) => <div className="inspector-row" data-field={field}>
    <label htmlFor={`${flipId}-${field}`} className="inspector-label fusion-flag-label">{label}</label>
    <span className="inspector-value">
      <input id={`${flipId}-${field}`} type="checkbox" checked={checked}
        onChange={event => { const on = event.target.checked; void commit(build((found, _keyFrame, where) => [{ op: 'set_fusion_node', ...where, id: found.id, params: { [field]: on } }])); }} />
    </span>
  </div>;

  const settings = labels.settings;
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
