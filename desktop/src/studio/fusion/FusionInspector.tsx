import { Fragment, useCallback, useId, useRef, useState, type MouseEvent as ReactMouseEvent, type ReactNode } from 'react';
import { ChevronLeft, ChevronRight, Eraser, Minus, Plus, Target, X } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import type { Menu, MenuItem } from '../color/ColorMenu';
import { CoveringMenu } from '../color/CoveringMenu';
import { COMPOSITES } from '../color/nodeEdits';
import { TrackButtons, type TrackButton } from '../color/TrackButtons';
import {
  EngineError, type Clip, type Composite, type Composition, type FusionBlurFilter, type FusionFillMethod, type FusionFollow, type FusionKeyerReplace,
  type FusionKeyerSpill, type FusionKeyerView, type FusionMaskCommon, type FusionNode, type FusionPaintMode, type FusionTrackerMerge, type FusionTrackerOperation,
  type FusionTrackerPattern, type FusionTrackerPivot, type Media, type Op, type Param, type Timeline,
} from '../engine';
import { useFairlightText } from '../fairlight/words';
import { frameOfKey, keyAt, keyFrameOf } from '../inspector/clipKeys';
import { ChoiceField, ColorField, FieldsLocked, KeyDiamond, NumberField, TextField } from '../inspector/fields';
import { refusalOf } from '../inspector/refusal';
import { useCommit, type Build } from '../inspector/useCommit';
import { valueAt } from '../keyframes/value';
import { clipLocations } from '../timeline/selection';
import { useAudioGesture } from '../timeline/useAudioGesture';
import {
  followOf, followOps, insertPoint, LEAST_POINTS, MOST_POINTS, MOST_TRACKERS, relabel, removePoint, shownComposition, withFirst, type FusionAt,
} from './edits';
import { isMaskKind, namesOf } from './graph';
import {
  addKey, isPointSetting, keyerRefusals, keyTimesOf, numbered, paramOf, pointPath, putValue, putValues, removeKey, settingOf, wholeParam, wholeValue,
  type FusionSetting,
} from './settings';
import {
  addPattern, clearPath, enablePattern, followChoices, followName, pathSamples, PATTERN_FIELDS, patternName, patternValue, removePattern, renamePattern,
  setPatternValue, setTracker, TRACKER_PLACE, type PatternField,
} from './tracker';
import type { FusionTracking } from './useFusionTracking';

/** How a setting is written and dragged: its decimals, how much a pixel of dragging changes it, its unit, and the range of its slider, where it has one. */
interface Shape { digits: number; scrub: number; unit?: string; slider?: { min: number; max: number; step: number } }

/** The ways that a mask is combined with the mask that comes into it, and the ways that a Polygon is filled, as the engine names them. */
const PAINT_MODES: readonly FusionPaintMode[] = ['merge', 'add', 'subtract'];
const FILLS: readonly FusionFillMethod[] = ['alternate', 'non_zero_winding'];
/** The filters of a Blur, as the engine names them. */
const FILTERS: readonly FusionBlurFilter[] = ['gaussian', 'box'];
/** What a Delta Keyer shows, what it fills what the key took away with, and how it suppresses the spill, as the engine names them and in the order of Fusion's menus. */
const VIEWS: readonly FusionKeyerView[] = ['pre_matte', 'matte', 'status', 'final_result'];
const REPLACES: readonly FusionKeyerReplace[] = ['none', 'source', 'hard_color', 'soft_color'];
const SPILLS: readonly FusionKeyerSpill[] = ['none', 'rare', 'medium', 'well_done', 'burnt'];
/** What a Tracker does with its pictures, which pictures it draws, and what its turn and its size are about, as the engine names them and in the order of Fusion's menus. */
const OPERATIONS: readonly FusionTrackerOperation[] = ['none', 'match_move'];
const TRACKER_MERGES: readonly FusionTrackerMerge[] = ['bg_only', 'fg_only', 'fg_over_bg', 'bg_over_fg'];
const PIVOTS: readonly FusionTrackerPivot[] = ['tracker_average', 'selected_tracker', 'manual'];
/**
 * The buttons that track the patterns of a Tracker, in the order of Fusion's (Track Reverse, one frame reverse, Stop, one frame
 * forward, Track Forward), and Track Forward and Back, which the Tracker palette of the Color page has too. Each tracks from the
 * playhead.
 */
const TRACK_BUTTONS: readonly TrackButton[] = ['reverse', 'one_reverse', 'stop', 'one_forward', 'forward', 'forward_and_back'];
/** The switches of the nodes, which take no keys: each a field of the `params` of its node. */
type Flag = 'flip_horizontal' | 'flip_vertical' | 'invert' | 'solid' | 'lock_xy' | 'whole_cells_only' | 'garbage_invert' | 'position' | 'rotation' | 'scaling';

/** Whether two follows are the same: of the same node, the same output, and the same pattern. */
const sameFollow = (a: FusionFollow, b: FusionFollow): boolean => a.node === b.node && a.output === b.output && a.tracker === b.tracker;
/** The modes of a Delta Keyer, which take no keys: each a field of its `params` that is one of a few names. */
type KeyerMode = 'view' | 'replace' | 'spill_method';
/** The two colours of a Delta Keyer, each three settings: its `r`, its `g` and its `b`. */
type KeyerColor = 'screen' | 'replace_color';

/**
 * How each kind of setting is shown, by its unit in the catalog. A place is a share of the picture,
 * with a slider over the picture itself (the field takes what lies outside it); a length that may
 * be less than nothing (the border of a mask) has a slider from one width of the picture the one
 * way to one the other; a number of a point of a Polygon has no slider, so that a list of points
 * stays short; an angle is in degrees, with a slider of one turn; a size has a slider up to five
 * times; every other setting is a number from 0 to 1.
 *
 * The settings of a Blur and of a Mosaic have ranges far wider than what is used day by day, so the
 * slider of each is over that part and the field takes the rest (`NumberField` keeps the two
 * apart): a Blur Size, in pixels of a picture 1080 high, has a slider to 100, as Fusion's has, of
 * its 500; the Pixel Frequency of a Mosaic one from 1 to 256 of its 16384; its Aspect one from a
 * quarter to four times of its sixteenth to sixteen; and an offset of its grid, in pixels of the
 * picture either way, is a field alone, since a slider of 16384 pixels each way would set nothing.
 *
 * So it is with a Delta Keyer. A channel of its screen colour or of its Replace Color may be as
 * much as a half float holds (65504), for a colour of a picture of high dynamic range: its slider
 * is from 0 to 1, and the field takes what lies above. Its Gain has a slider from 0 to 4 of its 10:
 * a Gain of 4 keys a screen a quarter as bright as the colour picked. Its lengths (Pre-Blur and
 * Matte Blur, each to 100 pixels of a picture 1080 high, and Erode/Dilate, 100 either way) have
 * sliders to 20, and from 20 the one way to 20 the other: a matte is seldom moved by more.
 */
function shapeOf(setting: FusionSetting): Shape {
  if (isPointSetting(setting)) return { digits: 3, scrub: 0.002 };
  if (setting.unit === 'widths' || setting.unit === 'heights') {
    const least = setting.bounds.low < 0 && !setting.path.startsWith('center.') ? -1 : 0;
    return { digits: 3, scrub: 0.002, slider: { min: least, max: 1, step: 0.001 } };
  }
  if (setting.unit === 'degrees_counterclockwise') return { digits: 1, scrub: 0.5, unit: '°', slider: { min: -180, max: 180, step: 0.1 } };
  // The lengths of a Delta Keyer, whose ranges end at 100, and a Blur Size, whose range ends at 500.
  if (setting.unit === 'pixels1080' && setting.bounds.high <= 100) return { digits: 1, scrub: 0.1, slider: { min: setting.bounds.low < 0 ? -20 : 0, max: 20, step: 0.1 } };
  if (setting.unit === 'pixels1080') return { digits: 1, scrub: 0.1, slider: { min: 0, max: 100, step: 0.1 } };
  if (setting.path.startsWith('screen.') || setting.path.startsWith('replace_color.')) return { digits: 3, scrub: 0.005, slider: { min: 0, max: 1, step: 0.001 } };
  if (setting.path === 'gain') return { digits: 3, scrub: 0.005, slider: { min: 0, max: 4, step: 0.01 } };
  if (setting.unit === 'pixels') return { digits: 1, scrub: 1, unit: 'px' };
  if (setting.path === 'frequency') return { digits: 1, scrub: 0.5, slider: { min: 1, max: 256, step: 1 } };
  if (setting.path === 'aspect') return { digits: 3, scrub: 0.005, slider: { min: 0.25, max: 4, step: 0.01 } };
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
 * - a Blur (Reference Manual 20, Ch. 93): Filter, Lock X/Y, Blur Size and Blend. While its X and Y
 *   are locked the Blur Size is one number, the `size.x` of the node, which the engine blurs across
 *   and down by; unlocked it is two, X Blur Size and Y Blur Size. Lock X/Y sets the switch and
 *   nothing else: the `size.y` of the node is kept as it is, and read again once it is unlocked;
 * - a Mosaic (the Mosaic Blur of Resolve FX, p. 3465): Pixel Frequency, Aspect, X Offset and
 *   Y Offset, and Whole Cells Only, which says where the pointer rests on it that it needs an
 *   effect mask;
 * - a Delta Keyer (Reference Manual 20, pp. 2440–2448): View Mode first, and then its settings in
 *   three sections, which are the tabs of Fusion's: Key (Background Color, the colour of the screen,
 *   as one colour and as its three channels, Gain, Balance and Pre-Blur), Matte (the two Thresholds,
 *   Erode/Dilate, Matte Blur, Clean Foreground and Clean Background, Replace Mode and, while that
 *   mode reads it, Replace Color as one colour and as its three channels, and the Invert of the
 *   garbage matte, which says where the pointer rests on it that it needs one linked) and Fringe
 *   (Spill Method and Spill Suppression). A colour chosen as one sets its channels together
 *   (`putValues`), so that the engine never checks a screen colour half way between two. The screen
 *   colour is not picked in the viewer yet, though the viewer can show the picture that comes into
 *   the keyer (the node before it, viewed);
 * - a Tracker (Reference Manual 20, pp. 2773–2784; PROTOCOL.md, "The Tracker"), in two sections,
 *   which are the tabs of Fusion's: Trackers, the buttons that track its enabled patterns from the
 *   playhead with what the track says (`useFusionTracking`; while it goes none of the settings of
 *   the Tracker is changed), the list of its patterns, each with its switch, its
 *   name, how many frames its path has and which frames of the timeline it reaches from and to, and
 *   a button that takes it away; Add Tracker (a pattern in the middle of the picture, at most 16);
 *   and of the pattern chosen in the list its name, where it was put, how large it and its search
 *   area are, its offset, and Clear Path. Each sets the list whole, the other patterns and every path
 *   as they are (`tracker.ts`). Operation: Operation, Merge (read with Match Move alone, and off
 *   otherwise), the three boxes of the motion, Pivot Type with the pattern or the place that it
 *   reads, and the Reference frame, shown and typed at a frame of the timeline and kept at the frame
 *   of the keys of the clip that the clip shows there, with Set to Playhead. None of it takes keys;
 * - a MediaIn and a MediaOut: nothing but the name.
 *
 * The centre of a Merge, a Transform and a mask has Connect To beside it (PROTOCOL.md, "Following a
 * Tracker"): a menu of what the Trackers of the composition publish, the place of each of their
 * patterns and their Steady and Unsteady Positions, and Disconnect. While the centre follows one,
 * its X and Y are kept and not read: they are shown, off, with their buttons of keys, and the
 * Inspector says what the centre follows.
 *
 * Every number has the three buttons of keys beside it (to the key before, a key put or taken away
 * at the playhead, to the key after), as the Inspector of the Edit page has them. A setting that has
 * keys takes a value typed or dragged as a key at the playhead; one without is one number. A label
 * double-clicked puts the default back. A slider or a label dragged is one gesture: each move sets
 * the setting whole, the release ends it, and the whole is one step to undo. Every other change is
 * one transaction, worked out from the clip as the engine has it when it is sent, with the first
 * composition before it where the clip has none yet (`withFirst`). What the engine refuses is said
 * at the top; but not a change that changes nothing (`empty`), which is no failure. The two
 * refusals of a Delta Keyer that are of no one number, a screen colour with no channel above its
 * other two and a lower threshold above the upper one, are said in words of their own
 * (`keyerRefusals`).
 *
 * Where the clip lies on a locked track (`locked`), the settings are shown and none is changed, as
 * in the Inspector of the Edit page (Reference Manual 20, p. 762): every field, switch and button
 * that changes something is off, and the Inspector says why. The buttons that go to the key before
 * and after stay, since they move the playhead alone.
 *
 * The Effects tab of the Inspector of the Edit page shows the settings of the effects of a clip, which are nodes of its
 * composition, with these same controls (`embedded`): the settings alone, under the heading that the tab gives the effect,
 * without the name of the node, its Name field or the sentence of a locked track, which the tab has of its own.
 */
export function FusionInspector({ bridge, session, revision, timeline, clip, media, node, name, frame, onSeek, locked: onLockedTrack = false, tracking, embedded = false }: {
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
  /** Whether the clip lies on a locked track: its settings are shown, and none is changed. */
  locked?: boolean;
  /** The tracking of the patterns of a Tracker, the page's: its buttons, what it says, and the track going on. */
  tracking?: FusionTracking;
  /** Shown in the Effects tab of the Inspector of the Edit page: the settings of the node alone. */
  embedded?: boolean;
}) {
  // A Tracker whose patterns are tracked is changed by the track alone, which any other change of the project would stop: its
  // settings are shown, and none is changed, as on a locked track.
  const tracked = tracking !== undefined && tracking.running && node !== null && node.kind === 'tracker'
    && (tracking.node === null || tracking.node === node.id);
  const locked = onLockedTrack || tracked;
  const words = useWords();
  const t = useFairlightText();
  const labels = words.label.studio.fusionPage;
  const said = words.hint.studio.fusionPage;
  const flipId = useId();
  const at: FusionAt = { timeline: timeline.id, clip: clip.id };
  const has = clip.fusion !== undefined;
  const { commit: send, refusal, setRefusal } = useCommit({ bridge, revision, session, timeline });
  // What the Inspector says better than the general sentences: the refusals of a Delta Keyer that are of no one number.
  const overrides = keyerRefusals(said);
  /** One transaction of the Inspector; what the engine refuses of it is said in the words of the Inspector. */
  const commit = (made: Build) => send(made, { overrides });
  const failed = (error: unknown) => { if (!isEmpty(error)) setRefusal(refusalOf(error, words.hint.studio, overrides)); };
  const gesture = useAudioGesture({ bridge, session, revision, identity: `${timeline.id}/${clip.id}/fusion/${node?.id ?? ''}`, locked, onFailure: failed });
  /** The setting that is dragged, as it was when the drag began, and the frame of its keys that the drag is at. */
  const dragged = useRef<{ path: string; initial: Param; keyFrame: number } | null>(null);
  const now = useRef({ has });
  now.current = { has };
  /** The point of a Polygon that is chosen in the list of its points, from 0, or null: where Add Point puts one after, and what Remove Point takes away. */
  const [chosenPoint, setChosenPoint] = useState<number | null>(null);
  /** The pattern of a Tracker that is chosen in its list, from 0, or null for the first: the one whose numbers are shown. */
  const [chosenPattern, setChosenPattern] = useState<number | null>(null);
  /** The menu of Connect To, while it is open. */
  const [menu, setMenu] = useState<Menu | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);

  const problem = refusal && refusal.reason !== 'empty' && <div className="inspector-problem" role="alert" data-test="fusion-inspector-problem">
    <p>{refusal.text}</p>
    <button type="button" className="icon-btn" aria-label={words.label.close} onClick={() => setRefusal(null)}><X size={14} /></button>
  </div>;
  if (node === null) return <div className="inspector" data-test="fusion-inspector-body">{problem}<p className="caption" data-test="fusion-no-node">{said.chooseNode}</p></div>;

  // The node as the functions below read it: it is one from here on, which they are not told by the test above.
  const shown: FusionNode = node;
  const keyFrame = keyFrameOf(clip, timeline, media, frame);
  const id = shown.id;
  /**
   * One transaction about the node, worked out from the clip and the node as they are when it is sent: the node, the frame of the
   * keys of the clip at the playhead, the clip as the operations name it, and the composition that the node is of.
   */
  const build = (ops: (found: FusionNode, keyFrameNow: number, where: FusionAt, composition: Composition) => Op[]): Build => (current) => {
    const located = clipLocations(current).get(clip.id);
    const composition = located ? shownComposition(located.clip) : undefined;
    const found = composition?.nodes.find(each => each.id === id);
    // A track that is locked when the change goes keeps its clips as they are, whatever asked for the change.
    if (!located || !composition || !found || located.track.locked) return [];
    const where = { timeline: current.id, clip: clip.id };
    return withFirst(where, located.clip.fusion !== undefined, ops(found, keyFrameOf(located.clip, current, media, frame), where, composition));
  };
  /** The three buttons of a setting: to the key before, a key put or taken away at the playhead, to the key after; all three off with `off`. */
  const keys = (setting: FusionSetting, label: string, off = false) => {
    const param = paramOf(shown, setting);
    const hit = keyAt(param, keyFrame) !== null;
    const times = keyTimesOf(clip, timeline, media, param);
    const go = (direction: -1 | 1) => {
      const target = direction < 0 ? times.filter(time => time < frame).at(-1) : times.find(time => time > frame);
      if (target !== undefined) onSeek(target, false);
    };
    return <div className="audio-param-keys" role="group" aria-label={label} data-keys={setting.path}>
      <button type="button" className="icon-btn" aria-label={`${label}: ${t('Previous keyframe')}`} title={t('Previous keyframe')}
        disabled={off || !times.some(time => time < frame)} onClick={() => go(-1)}><ChevronLeft size={14} aria-hidden="true" /></button>
      <button type="button" className="icon-btn" aria-label={`${label}: ${t(hit ? 'Remove keyframe' : 'Add keyframe')}`} title={t(hit ? 'Remove keyframe' : 'Add keyframe')}
        aria-pressed={hit} data-keyed={typeof param === 'object' || undefined} data-hit={hit || undefined} disabled={locked || off}
        onClick={() => void commit(build((found, keyFrameNow, where) => {
          const there = settingOf(found, setting.path);
          if (!there) return [];
          return keyAt(paramOf(found, there), keyFrameNow) ? removeKey(where, found, there, keyFrameNow) : addKey(where, found, there, keyFrameNow);
        }))}><KeyDiamond /></button>
      <button type="button" className="icon-btn" aria-label={`${label}: ${t('Next keyframe')}`} title={t('Next keyframe')}
        disabled={off || !times.some(time => time > frame)} onClick={() => go(1)}><ChevronRight size={14} aria-hidden="true" /></button>
    </div>;
  };

  /** A number of the node with its slider and its buttons of keys; with `off`, shown and not changed (a centre that follows a Tracker). */
  const number = (path: string, label: string, off = false): ReactNode => {
    const setting = settingOf(shown, path);
    if (!setting) return null;
    const shape = shapeOf(setting);
    const param = paramOf(shown, setting);
    return <Fragment key={path}>
      <NumberField name={path} label={label} value={valueAt(param, keyFrame)} fallback={setting.bounds.neutral} min={setting.bounds.low} max={setting.bounds.high}
        digits={shape.digits} scrub={shape.scrub} unit={shape.unit} slider={shape.slider} disabled={off}
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
      {keys(setting, label, off)}
    </Fragment>;
  };

  /**
   * A switch of the node that takes no keys: a flip of a Transform, the Invert and the Solid of a mask, the Lock X/Y of a Blur,
   * the Whole Cells Only of a Mosaic, the Invert of the garbage matte of a Delta Keyer and the three boxes of the motion of a
   * Tracker. `title` says more of it where the pointer rests on it.
   */
  const flag = (field: Flag, label: string, checked: boolean, title?: string) => <div className="inspector-row" data-field={field} title={title}>
    <label htmlFor={`${flipId}-${field}`} className="inspector-label fusion-flag-label">{label}</label>
    <span className="inspector-value">
      <input id={`${flipId}-${field}`} type="checkbox" checked={checked} disabled={locked}
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
  /** A setting of the node at the playhead: a number where it has no keys, and what its keys make it there. */
  const channel = (path: string) => { const setting = settingOf(shown, path); return setting ? valueAt(paramOf(shown, setting), keyFrame) : 0; };
  /** A mode of a Delta Keyer, which takes no keys: one of a few names, set by a patch of the node where it is another. */
  const keyerMode = <T extends string>(field: KeyerMode, label: string, value: T, options: readonly T[], names: Record<T, string>): ReactNode => <ChoiceField<T>
    name={field} label={label} value={value} options={options.map(each => ({ value: each, label: names[each] }))}
    onCommit={chosen => void commit(build((found, _keyFrame, where) => (found.kind === 'delta_keyer' && (found.params[field] as string) !== chosen
      ? [{ op: 'set_fusion_node', ...where, id: found.id, params: { [field]: chosen } }] : [])))} />;
  /**
   * A colour of a Delta Keyer as one: its red, green and blue are set together, those without keys by one patch of the node and
   * those with keys each where a value at the playhead goes (`putValues`). A channel above 1 is shown as 1: the field of the
   * channel has it whole.
   */
  const keyerColor = (name: KeyerColor, label: string): ReactNode => {
    const paths = [`${name}.r`, `${name}.g`, `${name}.b`];
    return <ColorField name={name} label={label} value={`#${paths.map(path => hex(channel(path))).join('')}ff`} onCommit={(chosen) => {
      const shares = [1, 3, 5].map(from => Number((Number.parseInt(chosen.slice(from, from + 2), 16) / 255).toFixed(3)));
      void commit(build((found, keyFrameNow, where) => putValues(where, found, paths.flatMap((path, index) => {
        const there = settingOf(found, path);
        return there ? [{ setting: there, value: shares[index] }] : [];
      }), keyFrameNow)));
    }} />;
  };

  // What the centre of the node follows of a Tracker, and the words that name it (Connect To).
  const composition = shownComposition(clip);
  const following = followOf(shown);
  const followWords = { unnamed: settings.tracker, steady: settings.steadyPosition, unsteady: settings.unsteadyPosition };
  const followed = following !== null ? followName(composition, namesOf(composition, labels.kinds), following, followWords) : null;
  /** The centre of a Merge, a Transform or a mask, its X and its Y off while it follows a Tracker, and Connect To beside it. */
  const centre = (): ReactNode => <>
    {number('center.x', settings.centerX, following !== null)}
    {number('center.y', settings.centerY, following !== null)}
    {connect()}
  </>;
  /**
   * Connect To: a button that says what the centre follows, which opens the menu of what the Trackers of the composition publish
   * (`followChoices`), the one followed checked, and Disconnect; and, while it follows one, what it follows.
   */
  const connect = (): ReactNode => {
    const open = (event: ReactMouseEvent<HTMLButtonElement>) => {
      const box = event.currentTarget.getBoundingClientRect();
      const names = namesOf(composition, labels.kinds);
      const items: MenuItem[] = followChoices(composition).map((each, place): MenuItem => ({
        id: `follow-${place}`, label: followName(composition, names, each, followWords), checked: following !== null && sameFollow(following, each),
        run: () => void commit(build((found, _keyFrame, where) => followOps(where, found, each))),
      }));
      if (items.length === 0) items.push({ id: 'follow-none', label: settings.noTrackers, disabled: true });
      items.push({ id: 'disconnect', label: settings.disconnect, disabled: following === null, run: () => void commit(build((found, _keyFrame, where) => followOps(where, found, null))) });
      setMenu({ x: box.left, y: box.bottom, label: settings.connectTo, items });
    };
    return <Fragment key="connect">
      <div className="inspector-row fusion-connect" data-test="fusion-connect">
        <span className="inspector-label">{settings.connectTo}</span>
        <span className="inspector-value">
          <button type="button" className="color-tool fusion-connect-button" aria-haspopup="menu" aria-label={`${settings.connectTo}: ${followed ?? settings.notConnected}`}
            title={followed ?? settings.notConnected} disabled={locked} onClick={open} data-follows={following?.node} data-test="connect-to">
            <Target size={13} aria-hidden="true" /><span className="fusion-connect-name">{followed ?? settings.notConnected}</span></button>
        </span>
      </div>
      {followed !== null && <p className="caption fusion-follows" data-test="fusion-follows">{settings.follows.replace('{name}', () => followed ?? '')}</p>}
    </Fragment>;
  };

  /**
   * The settings of a Tracker, none of which takes keys: its patterns (Trackers), and what it does with them (Operation). A change
   * of a pattern sends the list whole, worked out from the node as it is when it goes (`tracker.ts`).
   */
  const trackerBody = (params: Extract<FusionNode, { kind: 'tracker' }>['params']): ReactNode => {
    const patterns = Array.isArray(params.trackers) ? params.trackers : [];
    const count = patterns.length;
    // The pattern chosen, where the Tracker still has it; else the first.
    const chosen = count === 0 ? null : chosenPattern !== null && chosenPattern < count ? chosenPattern : 0;
    const pattern = chosen === null ? null : patterns[chosen];
    const nameOf = (index: number) => patternName(patterns[index], index, settings.tracker);
    /** How many frames the path of a pattern has, and the frames of the timeline at which the clip shows the first and the last of them. */
    const pathLine = (each: FusionTrackerPattern): string => {
      const samples = pathSamples(each.path);
      if (samples.length === 0) return settings.noPath;
      const first = String(frameOfKey(clip, timeline, media, samples[0][0]));
      if (samples.length === 1) return settings.pathOne.replace('{first}', () => first);
      const last = String(frameOfKey(clip, timeline, media, samples[samples.length - 1][0]));
      return settings.pathFrames.replace('{count}', () => String(samples.length)).replace('{first}', () => first).replace('{last}', () => last);
    };
    /** A number of the pattern chosen: where it was put, how large it or its search area is, or its offset. */
    const patternNumber = (index: number, field: PatternField, label: string): ReactNode => {
      const { neutral, low, high } = PATTERN_FIELDS[field];
      return <NumberField key={field} name={field} label={label} value={patternValue(patterns[index], field)} fallback={neutral} min={low} max={high}
        digits={3} scrub={0.002} slider={{ min: field.startsWith('offset.') ? -1 : 0, max: 1, step: 0.001 }}
        onCommit={value => void commit(build((found, _keyFrame, where) => setPatternValue(where, found, index, field, value)))} />;
    };
    /** A place of the pivot given by hand: its X or its Y. */
    const pivotNumber = (axis: 'x' | 'y', label: string): ReactNode => <NumberField key={`pivot_point.${axis}`} name={`pivot_point.${axis}`} label={label}
      value={params.pivot_point[axis]} fallback={TRACKER_PLACE.neutral} min={TRACKER_PLACE.low} max={TRACKER_PLACE.high} digits={3} scrub={0.002}
      slider={{ min: 0, max: 1, step: 0.001 }}
      onCommit={value => void commit(build((found, _keyFrame, where) => (found.kind === 'tracker'
        ? setTracker(where, found, { pivot_point: { ...found.params.pivot_point, [axis]: value } }) : [])))} />;
    // The patterns that the pivot may name: those that the Tracker has, and the one that it names, where that is not among them.
    const pivotPlaces = [...new Set([...patterns.map((_, index) => index), params.pivot_tracker])].sort((a, b) => a - b);
    const lastFrame = clip.start + Math.max(clip.length, 1) - 1;
    // What tracking says of this Tracker, or of whichever is chosen where the page took over a track that it did not start.
    const trackStatus = tracking?.status ?? null;
    const trackSaid = trackStatus !== null && (trackStatus.node === null || trackStatus.node === shown.id) ? trackStatus.text : null;
    return <>
      <h3 className="inspector-section">{labels.trackerTabs.trackers}</h3>
      {/* The enabled patterns tracked from the playhead; a clip on a locked track is not tracked, and Stop stops a track that goes on. */}
      {tracking && <div className="fusion-track" data-test="fusion-track">
        <TrackButtons running={tracking.running} buttons={TRACK_BUTTONS} disabled={onLockedTrack} onStart={tracking.start} onStop={tracking.stop} />
        {trackSaid !== null && <p className="caption fusion-track-status" role="status" data-test="fusion-track-status">{trackSaid}</p>}
      </div>}
      <div className="fusion-points-tools" role="group" aria-label={labels.trackerTabs.trackers}>
        {/* A pattern in the middle of the picture, after the others: it is the one chosen then. */}
        <button type="button" className="color-tool" disabled={locked || count >= MOST_TRACKERS} data-test="add-pattern"
          onClick={() => {
            void commit(build((found, _keyFrame, where) => addPattern(where, found))).then((sent) => { if (sent) setChosenPattern(count); });
          }}><Plus size={14} aria-hidden="true" />{settings.addTracker}</button>
      </div>
      {count === 0 && <p className="caption" data-test="fusion-no-patterns">{said.noTrackers}</p>}
      <ol className="fusion-patterns" data-test="fusion-patterns">
        {patterns.map((each, index) => {
          const named = nameOf(index);
          return <li key={index} className="fusion-pattern" data-pattern={index} data-chosen={chosen === index || undefined} data-enabled={each.enabled || undefined}>
            <span className="fusion-pattern-head">
              <input type="checkbox" checked={each.enabled} disabled={locked} aria-label={`${settings.enabled}: ${named}`} title={settings.enabled} data-test="pattern-enabled"
                onChange={(event) => { const on = event.target.checked; void commit(build((found, _keyFrame, where) => enablePattern(where, found, index, on))); }} />
              <button type="button" className="fusion-point-name" aria-pressed={chosen === index} onClick={() => setChosenPattern(index)} data-test="pattern-name">{named}</button>
              <button type="button" className="icon-btn" aria-label={`${settings.removeTracker}: ${named}`} title={settings.removeTracker} disabled={locked} data-test="remove-pattern"
                onClick={() => {
                  setChosenPattern(null);
                  void commit(build((found, _keyFrame, where, current) => removePattern(where, current, found, index)));
                }}><X size={13} aria-hidden="true" /></button>
            </span>
            <span className="caption fusion-pattern-path" data-test="pattern-path">{pathLine(each)}</span>
          </li>;
        })}
      </ol>
      {pattern !== null && chosen !== null && <Fragment key={`pattern-${chosen}`}>
        <TextField name="tracker.name" label={settings.trackerName} value={pattern.name} allowEmpty
          onCommit={text => void commit(build((found, _keyFrame, where) => renamePattern(where, found, chosen, text.trim())))} />
        {patternNumber(chosen, 'pattern.x', settings.patternX)}
        {patternNumber(chosen, 'pattern.y', settings.patternY)}
        {patternNumber(chosen, 'pattern.width', settings.patternWidth)}
        {patternNumber(chosen, 'pattern.height', settings.patternHeight)}
        {patternNumber(chosen, 'search.width', settings.searchWidth)}
        {patternNumber(chosen, 'search.height', settings.searchHeight)}
        {patternNumber(chosen, 'offset.x', settings.offsetX)}
        {patternNumber(chosen, 'offset.y', settings.offsetY)}
        <div className="fusion-points-tools">
          {/* The path is what the tracking wrote: it is taken away whole, and the pattern is tracked again from where it was put. */}
          <button type="button" className="color-tool" disabled={locked || pathSamples(pattern.path).length === 0} data-test="clear-path"
            onClick={() => void commit(build((found, _keyFrame, where) => clearPath(where, found, chosen)))}>
            <Eraser size={14} aria-hidden="true" />{settings.clearPath}</button>
        </div>
      </Fragment>}
      <h3 className="inspector-section">{labels.trackerTabs.operation}</h3>
      <ChoiceField<FusionTrackerOperation> name="operation" label={settings.operation} value={params.operation}
        options={OPERATIONS.map(each => ({ value: each, label: labels.operations[each] }))}
        onCommit={operation => void commit(build((found, _keyFrame, where) => setTracker(where, found, { operation })))} />
      {/* The Merge is read by Match Move alone. */}
      <ChoiceField<FusionTrackerMerge> name="merge" label={settings.trackerMerge} value={params.merge} disabled={params.operation !== 'match_move'}
        options={TRACKER_MERGES.map(each => ({ value: each, label: labels.trackerMerges[each] }))}
        onCommit={merge => void commit(build((found, _keyFrame, where) => setTracker(where, found, { merge })))} />
      {flag('position', settings.position, params.position)}
      {flag('rotation', settings.rotation, params.rotation)}
      {flag('scaling', settings.scaling, params.scaling)}
      <ChoiceField<FusionTrackerPivot> name="pivot" label={settings.pivotType} value={params.pivot}
        options={PIVOTS.map(each => ({ value: each, label: labels.pivots[each] }))}
        onCommit={pivot => void commit(build((found, _keyFrame, where) => setTracker(where, found, { pivot })))} />
      {params.pivot === 'selected_tracker' && <ChoiceField<string> name="pivot_tracker" label={settings.pivotTracker} value={String(params.pivot_tracker)}
        options={pivotPlaces.map(index => ({ value: String(index), label: nameOf(index) }))}
        onCommit={value => void commit(build((found, _keyFrame, where) => setTracker(where, found, { pivot_tracker: Number(value) })))} />}
      {params.pivot === 'manual' && pivotNumber('x', settings.pivotX)}
      {params.pivot === 'manual' && pivotNumber('y', settings.pivotY)}
      {/* The reference frame is kept at a frame of the keys of the clip, and shown and typed at the frame of the timeline that shows it. */}
      <NumberField name="reference" label={settings.reference} value={frameOfKey(clip, timeline, media, params.reference)} fallback={clip.start}
        min={clip.start} max={lastFrame} digits={0} scrub={0.25}
        onCommit={value => {
          const reference = keyFrameOf(clip, timeline, media, value);
          void commit(build((found, _keyFrame, where) => setTracker(where, found, { reference })));
        }} />
      <div className="fusion-points-tools">
        <button type="button" className="color-tool" disabled={locked} data-test="set-reference"
          onClick={() => void commit(build((found, keyFrameNow, where) => setTracker(where, found, { reference: keyFrameNow })))}>
          {settings.setToPlayhead}</button>
      </div>
    </>;
  };

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
      {centre()}
      {number('size', settings.size)}
      {number('angle', settings.angle)}
    </>;
  } else if (node.kind === 'transform') {
    body = <>
      {centre()}
      {number('size', settings.size)}
      {number('angle', settings.angle)}
      {flag('flip_horizontal', settings.flipHorizontal, node.params.flip_horizontal)}
      {flag('flip_vertical', settings.flipVertical, node.params.flip_vertical)}
    </>;
  } else if (node.kind === 'background') {
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
      {centre()}
      {number('width', settings.width)}
      {number('height', settings.height)}
      {number('corner_radius', settings.cornerRadius)}
      {number('angle', settings.angle)}
    </>;
  } else if (node.kind === 'ellipse') {
    body = <>
      {maskControls(node.params)}
      {centre()}
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
      {centre()}
      {number('size', settings.size)}
      {number('angle', settings.angle)}
      <ChoiceField<FusionFillMethod> name="fill" label={settings.fill} value={node.params.fill}
        options={FILLS.map(each => ({ value: each, label: labels.fills[each] }))}
        onCommit={fill => void commit(build((found, _keyFrame, where) => (found.kind === 'polygon' && found.params.fill !== fill
          ? [{ op: 'set_fusion_node', ...where, id: found.id, params: { fill } }] : [])))} />
      <h3 className="inspector-section">{settings.points}</h3>
      <div className="fusion-points-tools" role="group" aria-label={settings.points}>
        {/* After the point chosen, or after the last: the new point is the one chosen then, so that points are added one after the other. */}
        <button type="button" className="color-tool" disabled={locked || count >= MOST_POINTS} data-test="add-point"
          onClick={() => {
            const after = chosen;
            void commit(build((found, keyFrameNow, where) => insertPoint(where, found, after, keyFrameNow))).then((sent) => {
              if (sent) setChosenPoint(after === null ? count : after + 1);
            });
          }}><Plus size={14} aria-hidden="true" />{settings.addPoint}</button>
        {/* The point chosen, or the last. The points that are left keep their numbers and their keys. */}
        <button type="button" className="color-tool" disabled={locked || count <= LEAST_POINTS} data-test="remove-point"
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
  } else if (node.kind === 'blur') {
    const locked = node.params.lock_xy;
    body = <>
      <ChoiceField<FusionBlurFilter> name="filter" label={settings.filter} value={node.params.filter}
        options={FILTERS.map(each => ({ value: each, label: labels.filters[each] }))}
        onCommit={filter => void commit(build((found, _keyFrame, where) => (found.kind === 'blur' && found.params.filter !== filter
          ? [{ op: 'set_fusion_node', ...where, id: found.id, params: { filter } }] : [])))} />
      {flag('lock_xy', settings.lockXY, locked)}
      {/* Locked, the one Blur Size is `size.x`, which the engine blurs across and down by; `size.y` is kept and not shown. */}
      {number('size.x', locked ? settings.blurSize : settings.blurSizeX)}
      {!locked && number('size.y', settings.blurSizeY)}
      {number('blend', settings.blend)}
    </>;
  } else if (node.kind === 'mosaic') {
    body = <>
      {number('frequency', settings.pixelFrequency)}
      {number('aspect', settings.aspect)}
      {number('offset.x', settings.offsetX)}
      {number('offset.y', settings.offsetY)}
      {flag('whole_cells_only', settings.wholeCellsOnly, node.params.whole_cells_only, said.wholeCells)}
    </>;
  } else if (node.kind === 'delta_keyer') {
    const params = node.params;
    // The Replace Color is read by the two modes that fill with it: under the others it is kept, and not shown.
    const filled = params.replace === 'hard_color' || params.replace === 'soft_color';
    body = <>
      {keyerMode('view', settings.viewMode, params.view, VIEWS, labels.viewModes)}
      <h3 className="inspector-section">{labels.keyerTabs.key}</h3>
      {keyerColor('screen', settings.backgroundColor)}
      {number('screen.r', settings.screenRed)}
      {number('screen.g', settings.screenGreen)}
      {number('screen.b', settings.screenBlue)}
      {number('gain', settings.gain)}
      {number('balance', settings.balance)}
      {number('pre_blur', settings.preBlur)}
      <h3 className="inspector-section">{labels.keyerTabs.matte}</h3>
      {number('low', settings.thresholdLow)}
      {number('high', settings.thresholdHigh)}
      {number('erode_dilate', settings.erodeDilate)}
      {number('matte_blur', settings.matteBlur)}
      {number('clean_foreground', settings.cleanForeground)}
      {number('clean_background', settings.cleanBackground)}
      {keyerMode('replace', settings.replaceMode, params.replace, REPLACES, labels.replaceModes)}
      {filled && keyerColor('replace_color', settings.replaceColor)}
      {filled && number('replace_color.r', settings.replaceRed)}
      {filled && number('replace_color.g', settings.replaceGreen)}
      {filled && number('replace_color.b', settings.replaceBlue)}
      {/* The Invert of the garbage matte says where the pointer rests on it that it needs a garbage matte linked. */}
      {flag('garbage_invert', settings.garbageInvert, params.garbage_invert, said.garbageInvert)}
      <h3 className="inspector-section">{labels.keyerTabs.fringe}</h3>
      {keyerMode('spill_method', settings.spillMethod, params.spill_method, SPILLS, labels.spillMethods)}
      {number('spill_suppression', settings.spillSuppression)}
    </>;
  } else if (node.kind === 'tracker') {
    body = trackerBody(node.params);
  } else {
    body = <p className="caption" data-test="fusion-no-settings">{said.noSettings}</p>;
  }

  if (embedded) {
    return <div className="clip-effect-settings" data-test="clip-effect-settings" data-node={node.id} data-kind={node.kind}>
      {problem}
      <FieldsLocked.Provider value={locked}>
        <div className="inspector-panel" data-panel="fusion">{body}</div>
      </FieldsLocked.Provider>
      {menu && <CoveringMenu menu={menu} onClose={closeMenu} />}
    </div>;
  }
  return <div className="inspector" data-test="fusion-inspector-body" data-node={node.id} data-kind={node.kind}>
    {problem}
    <div className="inspector-what">
      <span className="inspector-name" data-test="fusion-node-name">{name}</span>
      <span className="caption">{labels.kinds[node.kind]}</span>
    </div>
    {/* Why the settings are off, in the words of the Inspector of the Edit page. */}
    {onLockedTrack && <p className="caption" data-test="fusion-inspector-locked">{words.hint.studio.inspector.locked}</p>}
    <FieldsLocked.Provider value={locked}>
      <div className="inspector-panel" data-panel="fusion">
        <TextField name="label" label={labels.nodeName} value={node.label} allowEmpty
          onCommit={label => void commit(build((found, _keyFrame, where) => relabel(where, { nodes: [found], links: [] }, found.id, label.trim())))} />
        {body}
      </div>
    </FieldsLocked.Provider>
    {menu && <CoveringMenu menu={menu} onClose={closeMenu} />}
  </div>;
}
