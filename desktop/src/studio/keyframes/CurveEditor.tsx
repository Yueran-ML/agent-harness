import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent, type PointerEvent, type ReactNode, type WheelEvent } from 'react';
import { ClipboardPaste, Copy, Maximize2 } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Keyframe, Param } from '../engine';
import {
  KEY_EASES, copyKeys, curvePath, deleteKeys, dragHandle, fitView, frameAt, handlePoints, hitCurve, hitHandle, hitKey, insertKey, interpIs, keyPoint,
  keysIn, moveKeys, pasteKeys, selectedIn, setInterp, spliceOf, ticksOf, valueAtY, xOf, yOf, zoomed,
  type CopiedKey, type KeyEase, type Point, type Range, type Rect, type Side, type View,
} from './curveGeometry';

/** How a value is written beside the graph: times `factor`, with `digits` places and a unit. */
export interface CurveDisplay { factor: number; unit: string; digits: number }
/**
 * The frames that a curve is drawn over, where they are not those of the keys of the settings: the
 * frames of a clip from its first, say. A curve over them has their ruler, their playhead and
 * their frames in use, and curves over the same frames are drawn together.
 */
export interface CurveClock {
  /** What tells these frames from others: curves whose clocks have one id are over the same frames. */
  id: string;
  /** The frame under the playhead, or null where there is none. */
  playhead: number | null;
  /** The frames that are in use, first and last: what lies outside is shaded, and a fitted view shows them. */
  range: readonly [number, number] | null;
  /** Move the playhead to a frame. */
  onSeek: (frame: number) => void;
}
/** What a drag of a point of a line leaves: its points, and what the graph says of it meanwhile, where that is not the value and the frame of the point. */
export interface LineDrag { keys: Keyframe[]; tip?: string }
/**
 * A curve that is no setting with keys but a line through points that whoever holds the editor
 * reads from something else (the Retime curves of a clip, Reference Manual 20, pp. 1078–1081). Its
 * points are the keys of the curve's `param`, each `linear` (a straight line to the next) or `hold`
 * (level to the next, and a step there), and its own rules say what a pointer does to them:
 * - a drag moves the one point pressed, to where `drag` puts it;
 * - Alt-click and a double click on the line ask for a point there (`add`), and Delete asks for the
 *   points chosen to be taken away (`remove`); whoever holds the editor does it, and the line is
 *   drawn again as it then is.
 * Such a line has no Bezier handles and is not eased, copied, pasted or nudged: the buttons of the
 * tool bar are off while it is the curve that is edited.
 */
export interface CurveLine {
  /** Whether it is shown before its box is ticked, and listed with the settings that have keys. */
  shown: boolean;
  /**
   * The points as a drag of the point at `index`, by `frames` along and by `value` up, leaves
   * them: as many as there were, in their order. Null where the point may not go there, and the
   * drag then stays where its last move took it. `initial` are the points as the drag began.
   */
  drag: (initial: readonly Keyframe[], index: number, frames: number, value: number) => LineDrag | null;
  /** Put a point on the line at a frame. */
  add?: (frame: number) => void;
  /** Take away the points at these frames. */
  remove?: (frames: readonly number[]) => void;
}
/**
 * A setting that the editor draws: its name, its colour, what it is, and the values that it takes.
 * With `line` it is a line with rules of its own, and with `clock` it is over frames of its own.
 */
export interface Curve { id: string; label: string; color: string; param: Param; bounds: Range; display?: CurveDisplay; clock?: CurveClock; line?: CurveLine }
/** What a setting is to be, after an edit. */
export interface CurveChange { id: string; param: Param }
/** The curve that is edited and its keys that are chosen, by their frames: a key keeps its frame where others come and go. */
export interface CurveSelection { curve: string | null; frames: readonly number[] }
export const NO_SELECTION: CurveSelection = { curve: null, frames: [] };
/** The keys that were copied, kept by whoever shares them between editors. */
export interface CurveClipboard { keys: CopiedKey[] }
const shared: CurveClipboard = { keys: [] };

/** The size of the graph until it is measured, and where it is not laid out. */
export const GRAPH_SIZE = { width: 640, height: 220 };
/** What the graph keeps free around its curves: the values at the left, the ruler of frames at the top. */
export const PLOT = { left: 52, top: 20, right: 10, bottom: 10 };
/** The rectangle of the curves in a graph of a size. */
export function plotRect(size: { width: number; height: number }): Rect {
  return { left: PLOT.left, top: PLOT.top, width: Math.max(1, size.width - PLOT.left - PLOT.right), height: Math.max(1, size.height - PLOT.top - PLOT.bottom) };
}

/** How far a pointer goes before a press is a drag. */
const SLOP_PX = 3;
/** The room, in pixels along the graph, that a value written at a point of a line takes: a point nearer the one written before it is not written. */
export const OWN_VALUE_GAP_PX = 44;
const DIAMOND = 'M0,-5 L5,0 L0,5 L-5,0 Z';
/** The faces of the buttons of interpolation: the way a curve leaves a key at the lower left and comes to one at the upper right. */
const GLYPHS: Record<KeyEase, string> = {
  linear: 'M2,13 L14,3',
  in: 'M2,13 C6,7 9,3 14,3',
  out: 'M2,13 C7,13 10,9 14,3',
  in_out: 'M2,13 C8,13 8,3 14,3',
  hold: 'M2,13 H8 V3 H14',
};

type Drag =
  | { kind: 'seek' }
  | { kind: 'box'; start: Point; add: boolean; before: readonly number[]; moved: boolean }
  | { kind: 'keys'; curve: Curve; view: View; start: Point; initial: Keyframe[]; indices: number[]; moved: boolean; axis: 'x' | 'y' | null; click: number | null; last: Param | null }
  | { kind: 'handle'; curve: Curve; view: View; start: Point; initial: Keyframe[]; index: number; side: Side; moved: boolean; last: Param | null };

/** Whether a curve is shown before its box is ticked: a setting that has keys, and a line that says so. */
const hasKeys = (curve: Curve) => (curve.line ? curve.line.shown : typeof curve.param !== 'number' && curve.param.keys.length > 0);
/** The frames that a curve is over, by the id of its clock: '' for those of the keys of the settings. */
const clockOf = (curve: Curve | null | undefined) => curve?.clock?.id ?? '';

/**
 * A curve editor for the keys of settings, as the Curve Editor of DaVinci Resolve's Edit page
 * (Reference Manual 20, Ch. 53, pp. 1116–1119). It calls no engine: it is given the settings and
 * says what they are to be. A drag says so at each move (`onPreview`) and once more as it ends
 * (`onCommit`, with what the last move said), or takes it back (`onCancel`); every other edit is one
 * `onCommit`. Whoever holds it makes a drag one step to undo.
 *
 * At the left are the settings, those with keys first, each with a box that shows or hides its
 * curve (the Parameters menu, p. 1119) and its name, which makes it the one that is edited. At the
 * right is the graph: a ruler in the frames of the keys (not of the timeline), the grid of values of
 * the curve that is edited, every curve shown (the others dimmed, each fitted to its own values),
 * their keys as diamonds, the Bezier handles of the keys chosen, and the playhead.
 *
 * What a pointer and the keyboard do:
 * - a click on a key chooses it; Ctrl or Cmd adds it or takes it out, Shift adds every key from the
 *   one clicked before to it (p. 1118); a click on a dimmed curve or on its key makes it the one
 *   that is edited (p. 1117);
 * - a drag from where nothing is draws a box and chooses the keys in it; with Shift, Ctrl or Cmd it
 *   adds them (p. 1118);
 * - a drag of a key chosen moves all that are chosen in time and in value; Shift keeps it to the
 *   direction that it took first (p. 1118);
 * - a drag of a Bezier handle shapes its stretch (p. 1118);
 * - Alt-click on a curve (p. 1117), or a double click, puts a key on it;
 * - Delete or Backspace takes the keys chosen away (p. 1118); Ctrl+C, Ctrl+X and Ctrl+V copy, cut
 *   and paste them, the first pasted at the playhead and the rest as far after it as they were
 *   (p. 1118); Ctrl+A chooses every key of the curve (p. 1118); Ctrl+← and Ctrl+→ move the keys
 *   chosen by a frame (p. 1111); Escape takes a drag back, or lets go of what is chosen;
 * - the tool bar sets the keys chosen to Linear, Ease In, Ease Out, Ease In and Out (p. 1119) or
 *   Hold, fits the view to the curves (Full Extent Zoom, p. 1120), and copies and pastes;
 * - a press on the ruler, and a drag along it, moves the playhead;
 * - the wheel zooms the frames about the pointer; with Shift it moves along them, and with Alt it
 *   zooms the values of the curve that is edited.
 * The keys that the editor takes do not go on to the page, whether or not anything is chosen: the
 * timeline would delete or copy clips with them. Every other key goes on, so the transport and Undo
 * work as everywhere.
 *
 * The view fits the curves shown until a key or a handle is dragged or the wheel is turned; from
 * then it stays, so that nothing moves under the pointer, until Full Extent Zoom.
 *
 * A curve may be over frames of its own (`CurveClock`), as the Retime curves of a clip are over the
 * frames of the clip and not over those of its keys. Curves over different frames cannot be laid
 * over one another: the graph draws the curves that are over the frames of the one that is edited,
 * with the ruler, the playhead and the frames in use of those frames, and keeps a view of each.
 * A curve may also be a line with rules of its own (`CurveLine`): its points are drawn and chosen
 * as keys are, one of them is dragged at a time, and the tool bar's buttons of interpolation, of
 * copying and of pasting are off for it, since its stretches are straight or level by what it is.
 *
 * Every curve has a scale of its own (`valuesOf`): a zoom about 1 and a place in pixels, or the
 * frames of a clip's media and its speed in percent, have no range in common, and one range for
 * both would press the smaller flat. The values written at the left of the graph are those of
 * the curve that is edited, so where another curve is drawn says nothing of its values by them.
 * A line that is drawn beside the one edited therefore says its own: the value of each of its
 * points is written at the point, in its colour (the Retime Speed curve says `48%` at its step
 * while the Retime Frame curve is edited, and the other way round).
 *
 * Left for the Spline Editor of the Fusion page, which is to be this component with more around
 * it: B-spline and loops (`Curve` would say how a setting goes on outside its keys), the Hand and
 * Add Keyframe modes and Show Handle of the tool bar (p. 1119), a zoom slider, tool tips over a
 * curve (p. 1120), snapping to the playhead, the track of keys beneath the graph (p. 1117), and a
 * window of its own (p. 1116). `tools` is where a page puts its own buttons.
 */
export function CurveEditor({ curves, playhead, selection, onSelect, range = null, disabled = false, clipboard = shared, title, tools, onPreview, onCommit, onCancel, onSeek }: {
  curves: readonly Curve[];
  /** The frame of keys under the playhead, or null where there is none. */
  playhead: number | null;
  /** What is chosen, where whoever holds the editor keeps it; left out, the editor keeps it. */
  selection?: CurveSelection;
  onSelect?: (selection: CurveSelection) => void;
  /** The frames of keys that are in use (those that a clip shows), first and last: what lies outside is shaded, and a fitted view shows them. */
  range?: readonly [number, number] | null;
  /** Nothing can be changed: the keys can still be chosen and copied, and the playhead moved. */
  disabled?: boolean;
  clipboard?: CurveClipboard;
  /** What the editor is called, where it is not just "Curve Editor". */
  title?: string;
  /** Buttons of the page at the end of the tool bar. */
  tools?: ReactNode;
  /** A drag as it goes: what the settings are to be for now. */
  onPreview: (changes: CurveChange[]) => void;
  /** An edit, or the end of a drag: what the settings are to be. */
  onCommit: (changes: CurveChange[]) => void;
  /** A drag taken back: the settings are to be what they were before its first `onPreview`. */
  onCancel: () => void;
  /** Move the playhead to a frame of keys. */
  onSeek: (keyFrame: number) => void;
}) {
  const words = useWords().label.studio.curves;
  const root = useRef<HTMLElement>(null);
  const svg = useRef<SVGSVGElement>(null);
  const clip = `curve-clip-${useId().replace(/[^a-zA-Z0-9]/g, '')}`;
  const [size, setSize] = useState(GRAPH_SIZE);
  const [own, setOwn] = useState<CurveSelection>(NO_SELECTION);
  const chosen = selection ?? own;
  const choose = (next: CurveSelection) => { setOwn(next); onSelect?.(next); };
  /** The curves shown or hidden by hand; the others are shown where they have keys. */
  const [ticked, setTicked] = useState<Record<string, boolean>>({});
  /** The frames shown of each clock, by its id, once its view no longer fits itself. */
  const [spans, setSpans] = useState<Record<string, { from: number; to: number }>>({});
  /** The values shown of each curve whose view no longer fits itself. */
  const [scales, setScales] = useState<Record<string, Range>>({});
  /** The curve that is dragged, as the drag has it. */
  const [live, setLive] = useState<CurveChange | null>(null);
  const [box, setBox] = useState<{ a: Point; b: Point } | null>(null);
  const [tip, setTip] = useState<string | null>(null);
  const [, redraw] = useState(0);
  const drag = useRef<Drag | null>(null);
  /** The key clicked last without Shift: where a Shift-click chooses from. */
  const anchor = useRef<number | null>(null);
  const cancelled = useRef(onCancel);
  cancelled.current = onCancel;

  useLayoutEffect(() => {
    const element = svg.current;
    if (!element) return undefined;
    const measure = () => {
      const { width, height } = element.getBoundingClientRect();
      if (width > 0 && height > 0) setSize(old => (old.width === width && old.height === height ? old : { width, height }));
    };
    measure();
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null;
    observer?.observe(element);
    return () => observer?.disconnect();
  }, []);
  // A drag that the editor is taken away under is taken back.
  useEffect(() => () => {
    const d = drag.current;
    drag.current = null;
    if (d && (d.kind === 'keys' || d.kind === 'handle') && d.last) cancelled.current();
  }, []);

  const listed = useMemo(() => [...curves.filter(hasKeys), ...curves.filter(curve => !hasKeys(curve))], [curves]);
  const visible = curves.filter(curve => ticked[curve.id] ?? hasKeys(curve));
  const active = visible.find(curve => curve.id === chosen.curve) ?? visible.find(hasKeys) ?? visible[0] ?? null;
  /** The frames that the graph is over: those of the curve that is edited. */
  const clock = active?.clock ?? null;
  const clockId = clockOf(active);
  /** The curves drawn: those shown that are over the frames of the graph. */
  const drawn = visible.filter(curve => clockOf(curve) === clockId);
  const head = clock ? clock.playhead : playhead;
  const used = clock ? clock.range : range;
  /** Whether the curve that is edited is a line with rules of its own. */
  const lined = Boolean(active?.line);
  const paramOf = (curve: Curve): Param => (live && live.id === curve.id ? live.param : curve.param);
  const keysOf = (curve: Curve): readonly Keyframe[] => { const param = paramOf(curve); return typeof param === 'number' ? [] : param.keys; };
  const rect = plotRect(size);
  const fitted = fitView(drawn.map(curve => curve.param), used ?? []);
  const frames = spans[clockId] ?? { from: fitted.from, to: fitted.to };
  const setSpan = (id: string, next: { from: number; to: number }) => setSpans(old => ({ ...old, [id]: next }));
  const valuesOf = (curve: Curve): Range => {
    const kept = scales[curve.id];
    if (kept) return kept;
    const fit = fitView([curve.param]);
    return { low: fit.low, high: fit.high };
  };
  const viewOf = (curve: Curve): View => ({ ...frames, ...valuesOf(curve) });
  /** The view of the frames alone, for the ruler and the playhead. */
  const ruler: View = { ...frames, low: 0, high: 1 };
  const selected = active && chosen.curve === active.id ? selectedIn(keysOf(active), chosen.frames) : [];
  /** The curves in the order in which a pointer finds them: the one that is edited first. */
  const order = active ? [active, ...drawn.filter(curve => curve.id !== active.id)] : drawn;
  const said = (curve: Curve, value: number) => {
    const display = curve.display;
    return display ? `${(value * display.factor).toFixed(display.digits)}${display.unit}` : String(Number(value.toFixed(3)));
  };

  /** Where a pointer is in the graph, in the pixels that it is drawn in. */
  const pointOf = (event: { clientX: number; clientY: number }): Point => {
    const bounds = svg.current?.getBoundingClientRect();
    const width = bounds && bounds.width > 0 ? bounds.width : size.width;
    const height = bounds && bounds.height > 0 ? bounds.height : size.height;
    return { x: ((event.clientX - (bounds?.left ?? 0)) * size.width) / width, y: ((event.clientY - (bounds?.top ?? 0)) * size.height) / height };
  };
  const capture = (event: PointerEvent<SVGSVGElement>) => {
    try { event.currentTarget.setPointerCapture?.(event.pointerId); } catch { /* A pointer that is no longer down has nothing to keep. */ }
  };
  const seek = (at: Point) => (clock ? clock.onSeek : onSeek)(Math.round(frameAt(ruler, rect, at.x)) + 0);
  const commit = (curve: Curve, param: Param) => onCommit([{ id: curve.id, param }]);

  /** Put a key on a curve where a pointer is, and choose it. */
  const addKey = (curve: Curve, at: Point) => {
    if (disabled) return;
    const frame = Math.round(frameAt(viewOf(curve), rect, at.x)) + 0;
    if (curve.line) {
      // Whoever holds the editor puts the point on its line: it is chosen once it is there.
      if (!curve.line.add) return;
      anchor.current = frame;
      choose({ curve: curve.id, frames: [frame] });
      curve.line.add(frame);
      return;
    }
    const made = insertKey(curve.param, frame);
    if (!made) return;
    anchor.current = frame;
    choose({ curve: curve.id, frames: [frame] });
    commit(curve, { keys: made.keys });
  };

  const down = (event: PointerEvent<SVGSVGElement>) => {
    if (event.button !== 0 || drag.current) return;
    event.preventDefault();
    root.current?.focus({ preventScroll: true });
    const at = pointOf(event);
    if (at.y < rect.top) { drag.current = { kind: 'seek' }; capture(event); seek(at); return; }
    const add = event.shiftKey || event.ctrlKey || event.metaKey;
    // A handle of a key chosen, which lies over the keys and the curves.
    if (active && !disabled) {
      const keys = keysOf(active);
      const view = viewOf(active);
      const handle = hitHandle(keys, selected, view, rect, at);
      if (handle) {
        drag.current = { kind: 'handle', curve: active, view, start: at, initial: keys.map(key => ({ ...key })), index: handle.index, side: handle.side, moved: false, last: null };
        capture(event);
        return;
      }
    }
    // A key: of the curve that is edited, or of another, which then is the one edited.
    for (const curve of order) {
      const keys = keysOf(curve);
      const view = viewOf(curve);
      const index = hitKey(keys, view, rect, at);
      if (index < 0) continue;
      const frame = keys[index].frame;
      const was = curve.id === chosen.curve ? chosen.frames : [];
      let next: number[];
      if (event.shiftKey && anchor.current !== null && curve.id === chosen.curve) {
        const low = Math.min(anchor.current, frame);
        const high = Math.max(anchor.current, frame);
        next = [...new Set([...was, ...keys.filter(key => key.frame >= low && key.frame <= high).map(key => key.frame)])];
      } else if (add) next = was.includes(frame) ? was.filter(each => each !== frame) : [...was, frame];
      else next = was.includes(frame) ? [...was] : [frame];
      if (!event.shiftKey) anchor.current = frame;
      choose({ curve: curve.id, frames: next });
      // A key taken out of what is chosen is not dragged.
      if (disabled || !next.includes(frame)) return;
      drag.current = {
        // Of a line with rules of its own, the one point pressed is dragged.
        kind: 'keys', curve, view, start: at, initial: keys.map(key => ({ ...key })), indices: curve.line ? [index] : selectedIn(keys, next), moved: false, axis: null,
        // A press on one of several chosen keeps them all for a drag; let go where it was, it chooses that one alone.
        click: !add && was.length > 1 && was.includes(frame) ? frame : null, last: null,
      };
      capture(event);
      return;
    }
    // A curve: Alt puts a key on it; a dimmed one becomes the one that is edited.
    for (const curve of order) {
      if (!hitCurve(paramOf(curve), viewOf(curve), rect, at)) continue;
      if (event.altKey) { addKey(curve, at); return; }
      if (curve.id !== active?.id) { anchor.current = null; choose({ curve: curve.id, frames: [] }); return; }
      break;
    }
    drag.current = { kind: 'box', start: at, add, before: add && active && chosen.curve === active.id ? [...chosen.frames] : [], moved: false };
    capture(event);
  };

  const move = (event: PointerEvent<SVGSVGElement>) => {
    const d = drag.current;
    if (!d) return;
    const at = pointOf(event);
    if (d.kind === 'seek') { seek(at); return; }
    let dx = at.x - d.start.x;
    let dy = at.y - d.start.y;
    if (!d.moved && Math.hypot(dx, dy) < SLOP_PX) return;
    if (d.kind === 'box') {
      d.moved = true;
      setBox({ a: d.start, b: at });
      if (active) {
        const keys = keysOf(active);
        const inside = keysIn(keys, viewOf(active), rect, d.start, at).map(index => keys[index].frame);
        choose({ curve: active.id, frames: [...new Set([...d.before, ...inside])] });
      }
      return;
    }
    if (!d.moved) {
      d.moved = true;
      // From here the view stays as it is: nothing is to move under the pointer.
      setSpan(clockOf(d.curve), { from: d.view.from, to: d.view.to });
      setScales(old => ({ ...old, [d.curve.id]: { low: d.view.low, high: d.view.high } }));
    }
    let keys: Keyframe[];
    if (d.kind === 'keys') {
      if (!event.shiftKey) d.axis = null;
      else if (d.axis === null) d.axis = Math.abs(dx) >= Math.abs(dy) ? 'x' : 'y';
      if (d.axis === 'x') dy = 0;
      else if (d.axis === 'y') dx = 0;
      const along = (dx / rect.width) * (d.view.to - d.view.from);
      const raise = (-dy / rect.height) * (d.view.high - d.view.low);
      // A point of a line goes where its rules put it, in time alone or in value alone until the pointer has left the other way by the slop of a press.
      const made: LineDrag | null = d.curve.line
        ? d.curve.line.drag(d.initial, d.indices[0], Math.abs(dx) < SLOP_PX ? 0 : along, Math.abs(dy) < SLOP_PX ? 0 : raise)
        : { keys: moveKeys(d.initial, d.indices, along, raise, d.curve.bounds) };
      if (!made) return;
      const moved = made.keys;
      choose({ curve: d.curve.id, frames: d.indices.map(index => moved[index].frame) });
      const first = moved[d.indices[0]];
      setTip(made.tip === undefined ? `${d.curve.label} ${said(d.curve, first.value)} · ${first.frame}` : `${d.curve.label} ${made.tip}`);
      keys = moved;
    } else {
      keys = dragHandle(d.initial, d.index, d.side, frameAt(d.view, rect, at.x), valueAtY(d.view, rect, at.y), d.curve.bounds);
    }
    // A move that leaves the keys as the move before left them says nothing again.
    if (d.last !== null && typeof d.last !== 'number' && spliceOf(d.last.keys, keys) === null) return;
    const param: Param = { keys };
    d.last = param;
    setLive({ id: d.curve.id, param });
    onPreview([{ id: d.curve.id, param }]);
  };

  const up = () => {
    const d = drag.current;
    if (!d) return;
    drag.current = null;
    if (d.kind === 'seek') return;
    if (d.kind === 'box') {
      setBox(null);
      // A click where nothing is lets go of what is chosen.
      if (!d.moved && !d.add) { anchor.current = null; choose({ curve: active?.id ?? null, frames: [] }); }
      return;
    }
    setLive(null);
    setTip(null);
    if (d.kind === 'keys' && !d.moved && d.click !== null) choose({ curve: d.curve.id, frames: [d.click] });
    if (d.last !== null) onCommit([{ id: d.curve.id, param: d.last }]);
  };

  /** Take a drag back. True where there was one. */
  const cancel = (): boolean => {
    const d = drag.current;
    if (!d) return false;
    drag.current = null;
    setLive(null); setBox(null); setTip(null);
    if (d.kind === 'box') choose({ curve: active?.id ?? null, frames: [...d.before] });
    if ((d.kind === 'keys' || d.kind === 'handle') && d.last !== null) {
      if (d.kind === 'keys') choose({ curve: d.curve.id, frames: d.indices.map(index => d.initial[index].frame) });
      onCancel();
    }
    return true;
  };

  const twice = (event: MouseEvent<SVGSVGElement>) => {
    const at = pointOf(event);
    if (at.y < rect.top) return;
    for (const curve of order) {
      const view = viewOf(curve);
      if (hitKey(keysOf(curve), view, rect, at) >= 0) return;
      if (hitCurve(paramOf(curve), view, rect, at)) { addKey(curve, at); return; }
    }
  };

  const wheel = (event: WheelEvent<SVGSVGElement>) => {
    if (drag.current) return;
    const by = event.deltaY || event.deltaX;
    if (by === 0) return;
    const at = pointOf(event);
    if (event.shiftKey) {
      const step = (by > 0 ? 0.1 : -0.1) * (frames.to - frames.from);
      setSpan(clockId, { from: frames.from + step, to: frames.to + step });
    } else if (event.altKey) {
      if (!active) return;
      const view = viewOf(active);
      const next = zoomed(view, by < 0 ? 1.25 : 0.8, { value: valueAtY(view, rect, at.y) });
      setScales(old => ({ ...old, [active.id]: { low: next.low, high: next.high } }));
    } else {
      const next = zoomed(ruler, by < 0 ? 1.25 : 0.8, { frame: frameAt(ruler, rect, at.x) });
      setSpan(clockId, { from: next.from, to: next.to });
    }
  };

  // --- The edits of the tool bar and of the keyboard, each one `onCommit`.
  const base: readonly Keyframe[] = active && typeof active.param !== 'number' ? active.param.keys : [];
  const picked = active && chosen.curve === active.id ? selectedIn(base, chosen.frames) : [];
  const able = active !== null && !disabled && picked.length > 0;
  const ease = (kind: KeyEase) => {
    if (!active || !able || lined || drag.current) return;
    const keys = setInterp(base, picked, kind);
    if (spliceOf(base, keys) !== null) commit(active, { keys });
  };
  const remove = () => {
    if (!active || !able || drag.current) return;
    if (active.line) {
      // Whoever holds the editor takes the points off its line.
      if (!active.line.remove) return;
      anchor.current = null;
      choose({ curve: active.id, frames: [] });
      active.line.remove(picked.map(index => base[index].frame));
      return;
    }
    const param = deleteKeys(active.param, picked, playhead ?? undefined);
    anchor.current = null;
    choose({ curve: active.id, frames: [] });
    commit(active, param);
  };
  const copy = (): boolean => {
    if (!active || lined || picked.length === 0) return false;
    clipboard.keys = copyKeys(base, picked);
    redraw(count => count + 1);
    return true;
  };
  const paste = () => {
    if (!active || lined || disabled || drag.current || playhead === null) return;
    const made = pasteKeys(active.param, clipboard.keys, playhead, active.bounds);
    if (!made) return;
    choose({ curve: active.id, frames: made.frames });
    if (typeof active.param === 'number' || spliceOf(active.param.keys, made.keys) !== null) commit(active, { keys: made.keys });
  };
  const nudge = (by: number) => {
    if (!active || !able || lined || drag.current) return;
    const keys = moveKeys(base, picked, by, 0, active.bounds);
    if (spliceOf(base, keys) === null) return;
    choose({ curve: active.id, frames: picked.map(index => keys[index].frame) });
    commit(active, { keys });
  };

  const keyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.nativeEvent.isComposing) return;
    const key = event.key.toLowerCase();
    const command = (event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey;
    const owned = key === 'escape' || key === 'delete' || key === 'backspace'
      || (command && (key === 'a' || key === 'c' || key === 'x' || key === 'v' || key === 'arrowleft' || key === 'arrowright'));
    if (!owned) return;
    // These are the editor's whether or not it has anything to do with them: the page would take them for the timeline.
    event.preventDefault();
    event.stopPropagation();
    if (key === 'escape') { if (!cancel() && active) { anchor.current = null; choose({ curve: active.id, frames: [] }); } return; }
    if (key === 'delete' || key === 'backspace') { remove(); return; }
    if (key === 'a') { if (active) choose({ curve: active.id, frames: base.map(each => each.frame) }); return; }
    if (key === 'c') { copy(); return; }
    if (key === 'x') { if (able && copy()) remove(); return; }
    if (key === 'v') { paste(); return; }
    nudge(key === 'arrowleft' ? -1 : 1);
  };

  const names: Record<KeyEase, string> = { linear: words.linear, in: words.easeIn, out: words.easeOut, in_out: words.easeInOut, hold: words.hold };
  const view = active ? viewOf(active) : null;
  const right = rect.left + rect.width;
  const bottom = rect.top + rect.height;
  const within = (x: number) => Math.min(right, Math.max(rect.left, x));
  const playheadX = head === null ? null : xOf(ruler, rect, head);
  return <section ref={root} className="curve-editor" role="group" aria-label={title ?? words.title} tabIndex={0} data-disabled={disabled || undefined}
    data-test="curve-editor" onKeyDown={keyDown}>
    {/* A button pressed with the pointer gives the keyboard back to the editor, so that its keys go on working (and Space does not press the button again). */}
    <header className="curve-editor-bar" onClick={(event) => { if (event.detail > 0) root.current?.focus({ preventScroll: true }); }}>
      <strong>{title ?? words.title}</strong>
      <div className="curve-editor-tools" role="group" aria-label={words.interpolation}>
        {KEY_EASES.map(kind => <button key={kind} type="button" className="icon-btn" aria-label={names[kind]} title={names[kind]} data-ease={kind}
          aria-pressed={!lined && interpIs(base, picked, kind)} disabled={!able || lined} onClick={() => ease(kind)}>
          <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true"><path d={GLYPHS[kind]} fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
        </button>)}
      </div>
      <button type="button" className="icon-btn" aria-label={words.fit} title={words.fit} data-test="curve-fit" onClick={() => { setSpans({}); setScales({}); }}><Maximize2 size={14} /></button>
      <button type="button" className="icon-btn" aria-label={words.copy} title={`${words.copy} (Ctrl+C)`} data-test="curve-copy" disabled={lined || picked.length === 0} onClick={() => { copy(); }}><Copy size={14} /></button>
      <button type="button" className="icon-btn" aria-label={words.paste} title={`${words.paste} (Ctrl+V)`} data-test="curve-paste"
        disabled={disabled || active === null || lined || playhead === null || clipboard.keys.length === 0} onClick={paste}><ClipboardPaste size={14} /></button>
      <span className="curve-editor-gap" />
      {tools}
    </header>
    <ul className="curve-editor-list" aria-label={words.parameters}>
      {listed.map(curve => <li key={curve.id} data-curve={curve.id} data-active={curve.id === active?.id || undefined}>
        <input type="checkbox" aria-label={curve.label} checked={ticked[curve.id] ?? hasKeys(curve)}
          onChange={event => { const shown = event.target.checked; setTicked(old => ({ ...old, [curve.id]: shown })); }} />
        <span className="curve-editor-swatch" style={{ background: curve.color }} aria-hidden="true" />
        <button type="button" className="curve-editor-name" aria-pressed={curve.id === active?.id}
          onClick={() => { setTicked(old => ({ ...old, [curve.id]: true })); anchor.current = null; choose({ curve: curve.id, frames: [] }); }}>{curve.label}</button>
        {hasKeys(curve) && <span className="curve-editor-keyed" aria-hidden="true">◆</span>}
      </li>)}
    </ul>
    <svg ref={svg} className="curve-editor-graph" aria-label={words.graph} viewBox={`0 0 ${size.width} ${size.height}`} preserveAspectRatio="none"
      onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={() => { cancel(); }} onLostPointerCapture={() => { cancel(); }}
      onDoubleClick={twice} onWheel={wheel}>
      <defs><clipPath id={clip}><rect x={rect.left} y={rect.top} width={rect.width} height={rect.height} /></clipPath></defs>
      <rect className="curve-editor-ruler" x={0} y={0} width={size.width} height={rect.top} />
      <rect className="curve-editor-plot" x={rect.left} y={rect.top} width={rect.width} height={rect.height} />
      {used && <>
        <rect className="curve-editor-outside" x={rect.left} y={rect.top} width={Math.max(0, within(xOf(ruler, rect, used[0])) - rect.left)} height={rect.height} />
        <rect className="curve-editor-outside" x={within(xOf(ruler, rect, used[1]))} y={rect.top} width={Math.max(0, right - within(xOf(ruler, rect, used[1])))} height={rect.height} />
      </>}
      {ticksOf(frames.from, frames.to, Math.max(2, Math.floor(rect.width / 72)), true).map(frame => {
        const x = xOf(ruler, rect, frame);
        return <g key={frame}>
          <line className="curve-editor-grid" x1={x} x2={x} y1={rect.top} y2={bottom} />
          <line className="curve-editor-tick" x1={x} x2={x} y1={rect.top - 5} y2={rect.top} />
          <text className="curve-editor-frame" x={x + 3} y={rect.top - 7}>{frame}</text>
        </g>;
      })}
      {active && view && ticksOf(view.low, view.high, Math.max(2, Math.floor(rect.height / 32))).map(value => {
        const y = yOf(view, rect, value);
        return <g key={value}>
          <line className="curve-editor-grid" x1={rect.left} x2={right} y1={y} y2={y} />
          <text className="curve-editor-value" x={rect.left - 6} y={y} textAnchor="end" dominantBaseline="middle">{said(active, value)}</text>
        </g>;
      })}
      <g clipPath={`url(#${clip})`}>
        {[...order].reverse().map(curve => <path key={curve.id} className="curve-line" data-curve={curve.id} data-active={curve.id === active?.id}
          d={curvePath(paramOf(curve), viewOf(curve), rect)} stroke={curve.color} fill="none" />)}
        {active && view && selected.flatMap((index) => {
          const keys = keysOf(active);
          const at = keyPoint(keys[index], view, rect);
          const handles = handlePoints(keys, index, view, rect);
          return (['in', 'out'] as const).flatMap((side) => {
            const point = handles[side];
            return point ? [<g key={`${index}-${side}`} className="curve-handle" data-key={index} data-side={side} style={{ color: active.color }}>
              <line x1={at.x} y1={at.y} x2={point.x} y2={point.y} />
              <circle r={3.5} transform={`translate(${point.x} ${point.y})`} />
            </g>] : [];
          });
        })}
        {[...order].reverse().map((curve) => {
          const on = curve.id === active?.id;
          const shown = viewOf(curve);
          return <g key={curve.id} className="curve-keys" data-curve={curve.id} data-active={on} style={{ color: curve.color }}>
            {keysOf(curve).map((key, index) => {
              const point = keyPoint(key, shown, rect);
              return <path key={index} className="curve-key" d={DIAMOND} transform={`translate(${point.x} ${point.y})`} data-curve={curve.id} data-key={index}
                data-frame={key.frame} data-selected={on && selected.includes(index)} />;
            })}
          </g>;
        })}
        {/* The values of a line that is not the one edited, at its points: the values at the left are not its own. */}
        {drawn.filter(curve => curve.line && curve.id !== active?.id).map((curve) => {
          const shown = viewOf(curve);
          // The place of the value written last: one that would lie on it is left out.
          let last = -Infinity;
          return <g key={curve.id} className="curve-own-values" data-curve={curve.id}>
            {keysOf(curve).flatMap((key, index) => {
              const point = keyPoint(key, shown, rect);
              if (point.x < rect.left || point.x > right || point.x - last < OWN_VALUE_GAP_PX) return [];
              last = point.x;
              // Beside the point and above it; under it at the upper edge of the graph, and at its left at the right edge.
              const atEnd = point.x > right - OWN_VALUE_GAP_PX;
              return [<text key={index} className="curve-editor-own-value" data-key={index} style={{ fill: curve.color }} textAnchor={atEnd ? 'end' : 'start'}
                x={point.x + (atEnd ? -7 : 7)} y={point.y + (point.y < rect.top + 16 ? 13 : -6)}>{said(curve, key.value)}</text>];
            })}
          </g>;
        })}
        {box && <rect className="curve-editor-box" x={Math.min(box.a.x, box.b.x)} y={Math.min(box.a.y, box.b.y)} width={Math.abs(box.b.x - box.a.x)} height={Math.abs(box.b.y - box.a.y)} />}
      </g>
      {playheadX !== null && playheadX >= rect.left && playheadX <= right
        && <line className="curve-editor-playhead" data-frame={head ?? undefined} x1={playheadX} x2={playheadX} y1={0} y2={size.height} />}
      {tip && <text className="curve-editor-tip" x={rect.left + 8} y={rect.top + 14}>{tip}</text>}
    </svg>
  </section>;
}
