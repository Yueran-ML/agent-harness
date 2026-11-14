import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent, type PointerEvent } from 'react';
import { Eraser, Eye, EyeOff, Lock, StepBack, StepForward, Trash2, Unlock } from 'lucide-react';
import { useWords } from '../../app/locale';
import { Sheet } from '../../app/Sheet';
import type { EngineBridge } from '../../bridge';
import type { Grade, GradeNode, Target } from '../engine';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { ColorMenu, type Menu, type MenuItem } from './ColorMenu';
import { numberText } from './graph';
import {
  DIAMOND, LANE, LANES_WIDTH, MARK_RADIUS, easePath, frameAtX, hitMark, keyFrameAt, keyRange, lanesHeight, marksOf, rowAt, rowTop, rulerTicks,
  timelineFrameOf, xOfFrame, type KeySpan, type Mark, type Point,
} from './keyTracks';
import { isLocked, toggleLock, toggleNodes } from './nodeEdits';
import {
  EASE_MOST, EASE_STEP, KEY_SCOPES, NODE_KEYED, easeOf, moveKeyframe, moveRange, nodesFor, tracksOf, withKeyframeMoved,
  type KeyKind, type KeyScope, type KeyTracks,
} from './nodeKeys';
import type { GradeEditing } from './useGradeEditing';
import type { KeyframeEditing } from './useKeyframes';
import './keyframes.css';

/** How far a pointer goes before a press on a mark is a drag. */
const SLOP_PX = 3;
/** A number of a mark's place: two places are finer than a screen shows. */
const n = (value: number) => Number(value.toFixed(2)) + 0;

/** The mark that is chosen: on the master track (`node` null) or on a node's, at a frame of the keys. */
interface Chosen { node: string | null; frame: number }

type Drag =
  | { kind: 'seek' }
  /** A press on a mark: the nodes whose keyframe a drag moves, as they were when it was pressed, and the frames that it may go to. */
  | { kind: 'mark'; mark: Mark; nodes: GradeNode[]; low: number; high: number; start: number; moved: boolean; to: number };

/** The face of a button that adds a keyframe: its mark, and a plus. */
function AddGlyph({ kind }: { kind: KeyKind }) {
  return <svg width={16} height={14} viewBox="0 0 16 14" aria-hidden="true" className="color-kf-glyph">
    {kind === 'dynamic' ? <path d="M5,2 L10,7 L5,12 L0,7 Z" /> : <circle cx={5} cy={7} r={4} />}
    <path className="color-kf-plus" d="M13,1 V7 M10,4 H16" />
  </svg>;
}

/** The marks of the tracks: drawn again only where the keyframes, the width or the mark chosen change, not as the playhead goes. */
const KeyMarks = memo(function KeyMarks({ marks, chosen, dynamic, held }: { marks: readonly Mark[]; chosen: Chosen | null; dynamic: string; held: string }) {
  return <g className="color-kf-marks">
    {marks.map((mark) => {
      const picked = (chosen !== null && chosen.node === mark.node && chosen.frame === mark.frame) || undefined;
      const place = `translate(${n(mark.x)} ${n(mark.y)})`;
      const name = `${mark.row}:${mark.frame}`;
      return mark.kind === 'dynamic'
        ? <path key={name} className="color-kf-mark" d={DIAMOND} transform={place} data-row={mark.row} data-node={mark.node ?? undefined} data-frame={mark.frame}
          data-kind="dynamic" data-mixed={mark.mixed || undefined} data-selected={picked}><title>{`${dynamic} · ${mark.frame}`}</title></path>
        : <circle key={name} className="color-kf-mark" r={MARK_RADIUS} transform={place} data-row={mark.row} data-node={mark.node ?? undefined} data-frame={mark.frame}
          data-kind="static" data-selected={picked}><title>{`${held} · ${mark.frame}`}</title></circle>;
    })}
  </g>;
});

/**
 * Change Dynamic Attributes (Reference Manual 20, pp. 3391–3392): the two eases of a dynamic
 * keyframe, Start as the settings leave it and End as they come to the next keyframe, each from 0
 * (none: a line) to `EASE_MOST`, with the curve that they make.
 */
function DynamicAttributes({ start, end, onApply, onClose }: { start: number; end: number; onApply: (start: number, end: number) => void; onClose: () => void }) {
  const words = useWords();
  const labels = words.label.studio.colorPage.keyframesPanel;
  const [from, setFrom] = useState(start);
  const [to, setTo] = useState(end);
  const box = { width: 160, height: 90 };
  return <Sheet title={labels.attributes} onClose={onClose} foot={<>
    <button type="button" className="btn" onClick={onClose}>{words.label.cancel}</button>
    <button type="button" className="btn btn-main" onClick={() => onApply(from, to)} data-test="kf-ease-apply">{words.label.apply}</button>
  </>}>
    <form className="color-kf-ease" onSubmit={(event) => { event.preventDefault(); onApply(from, to); }}>
      <label className="setting-name" htmlFor="kf-ease-start">{labels.start}</label>
      <input id="kf-ease-start" type="range" min={0} max={EASE_MOST} step={EASE_STEP} value={from} data-first
        onChange={event => setFrom(Number(event.target.value))} data-test="kf-ease-start" />
      <output className="mono" htmlFor="kf-ease-start">{from}</output>
      <label className="setting-name" htmlFor="kf-ease-end">{labels.end}</label>
      <input id="kf-ease-end" type="range" min={0} max={EASE_MOST} step={EASE_STEP} value={to}
        onChange={event => setTo(Number(event.target.value))} data-test="kf-ease-end" />
      <output className="mono" htmlFor="kf-ease-end">{to}</output>
      <svg className="color-kf-ease-curve" width={box.width} height={box.height} viewBox={`-4 -4 ${box.width + 8} ${box.height + 8}`}
        role="img" aria-label={labels.profile} data-test="kf-ease-curve">
        <path className="color-kf-ease-line" d={`M0,${box.height} L${box.width},0`} />
        <path className="color-kf-ease-path" d={easePath(from, to, box)} />
      </svg>
    </form>
  </Sheet>;
}

/**
 * The Keyframe Editor of the Color page (DaVinci Resolve Reference Manual 20, Ch. 148, pp. 3380–3394),
 * at the right of the palettes: the keyframes of the grade shown, a track for each of its nodes
 * beneath a master track that shows them all (p. 3382).
 *
 * - The ruler counts the frames of the keys of the grade (`keyTracks`); a press on it, and a drag
 *   along it, moves the playhead.
 * - A dynamic keyframe is a diamond and a static one a circle (pp. 3384–3385). A click on one
 *   chooses it and moves the playhead to it; a drag moves it in time, no further than the keyframes
 *   beside it, as one step to undo (each move sets the settings whole, the last ends the gesture,
 *   Escape takes it back). A mark of the master track stands for the keyframes of every node there.
 * - The tool bar has the mode, All or Color (pp. 3383–3384), Add Dynamic Keyframe, Add Static
 *   Keyframe, Delete Keyframe and Delete All Keyframes, which are for the tracks of the mode at the
 *   playhead, and the keyframe before and after the playhead.
 * - A right click on a mark changes it to static or dynamic (p. 3386), opens Change Dynamic
 *   Attributes (pp. 3391–3392) or deletes it; one on a track adds a keyframe to it at the playhead
 *   or deletes all of its keyframes.
 * - The head of a node's track names it as the node editor does (a click makes it the current
 *   node) and has its switches: the node on or off, Lock Node (p. 3382), and Auto-Keyframe (p. 3383),
 *   with which a change of a palette at a frame without a keyframe puts one there first.
 *
 * It keeps nothing of the keys: whatever changes the grade (a palette, Undo, the agent) is drawn as
 * it comes. Not here yet: the tracks of the groups of settings inside a node, the Sizing, FX and
 * Ext Matte tracks, copying and pasting keyframes, and EDL marks.
 */
export function KeyframesPanel({ bridge, session, revision, grade, target, current, onCurrent, span, frame, keys, editing, onSeek, onFailure, hint }: {
  bridge: Pick<EngineBridge, 'call'>;
  session: number;
  revision: number;
  /** The grade shown, with its default node where it has none; null where there is none. */
  grade: Grade | null;
  target: Target | null;
  /** The current node. */
  current: string | null;
  onCurrent: (node: string) => void;
  /** Where the grade is shown and the clock of its keys; null where nothing is. */
  span: KeySpan | null;
  /** The playhead, in frames of the timeline; null where no timeline is loaded. */
  frame: number | null;
  keys: KeyframeEditing;
  editing: Pick<GradeEditing, 'run'>;
  onSeek: (frame: number, scrubbing: boolean) => void;
  /** A refusal of a drag. */
  onFailure: (failure: unknown) => void;
  /** What is said where there is no grade to show. */
  hint: string;
}) {
  const words = useWords();
  const page = words.label.studio.colorPage;
  const labels = page.keyframesPanel;
  const root = useRef<HTMLElement>(null);
  const svg = useRef<SVGSVGElement>(null);
  const [width, setWidth] = useState(LANES_WIDTH);
  const [chosen, setChosen] = useState<Chosen | null>(null);
  /** The keyframe that is dragged, as the drag has it: the nodes as they were, and where their keyframe is put. */
  const [live, setLive] = useState<{ nodes: readonly GradeNode[]; from: number; to: number } | null>(null);
  const [menu, setMenu] = useState<Menu | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);
  const [easing, setEasing] = useState<{ on: KeyTracks; frame: number; start: number; end: number } | null>(null);
  const drag = useRef<Drag | null>(null);
  /** Whether a drag has begun a gesture that is still to be ended or taken back. */
  const dragging = useRef(false);
  const identity = JSON.stringify(target);
  const gesture = useAudioGesture({ bridge, session, revision, identity: `${identity}/keyframes`, locked: false, onFailure });
  // Another grade: nothing of the one before is chosen or open.
  useEffect(() => { setChosen(null); setMenu(null); setEasing(null); setLive(null); drag.current = null; dragging.current = false; }, [identity, session]);

  const shown = grade !== null && span !== null;
  useLayoutEffect(() => {
    const element = svg.current;
    if (!element) return undefined;
    const measure = () => {
      const measured = element.getBoundingClientRect().width;
      if (measured > 0) setWidth(old => (old === measured ? old : measured));
    };
    measure();
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null;
    observer?.observe(element);
    return () => observer?.disconnect();
  }, [shown]);

  // A keyframe that is dragged is drawn where the drag has it, from the nodes as they were when it began.
  const drawn = useMemo(() => {
    if (grade === null || live === null) return grade;
    const moved = new Map(live.nodes.map(node => [node.id, withKeyframeMoved(node, live.from, live.to)]));
    return { ...grade, nodes: grade.nodes.map(node => moved.get(node.id) ?? node) };
  }, [grade, live]);
  const tracks = useMemo(() => (drawn === null ? [] : tracksOf(drawn)), [drawn]);
  const range = useMemo(() => (span === null ? null : keyRange(span)), [span]);
  const marks = useMemo(() => (range === null ? [] : marksOf(tracks, range, width)), [tracks, range, width]);
  const ticks = useMemo(() => (range === null ? [] : rulerTicks(range, width)), [range, width]);

  if (grade === null || span === null || range === null) {
    return <section className="color-kf" aria-label={labels.title} data-test="keyframes-panel">
      <p className="caption color-empty" data-test="no-keyframes">{hint}</p>
    </section>;
  }

  // What is shown, by names that the handlers below keep as they are here.
  const shownGrade: Grade = grade;
  const shownSpan: KeySpan = span;
  const frames: readonly [number, number] = range;
  const playhead = frame === null ? null : keyFrameAt(shownSpan, frame);
  const height = lanesHeight(tracks.length);
  // The track that the tool bar is for: the master track in the All mode, the current node's in Color.
  const scoped = keys.scope === 'all' ? tracks[0] : tracks.find(track => track.node !== null && track.node === current);
  const here = playhead !== null && (scoped?.keyframes.some(each => each.frame === playhead) ?? false);
  const any = (scoped?.keyframes.length ?? 0) > 0;
  const can = target !== null && playhead !== null;

  /** Where a pointer is in the lanes, in the pixels that they are drawn in. */
  const pointOf = (event: { clientX: number; clientY: number }): Point => {
    const bounds = svg.current?.getBoundingClientRect();
    const scale = bounds && bounds.width > 0 ? width / bounds.width : 1;
    return { x: (event.clientX - (bounds?.left ?? 0)) * scale, y: event.clientY - (bounds?.top ?? 0) };
  };
  const capture = (event: PointerEvent<SVGSVGElement>) => {
    try { event.currentTarget.setPointerCapture?.(event.pointerId); } catch { /* A pointer that is no longer down has nothing to keep. */ }
  };
  const seekTo = (x: number, scrubbing: boolean) => onSeek(timelineFrameOf(shownSpan, frameAtX(frames, width, x)), scrubbing);
  const tracksOn = (node: string | null): KeyTracks => (node === null ? 'master' : { node });

  const down = (event: PointerEvent<SVGSVGElement>) => {
    if (event.button !== 0 || drag.current) return;
    event.preventDefault();
    root.current?.focus({ preventScroll: true });
    const at = pointOf(event);
    if (at.y < LANE.ruler) { drag.current = { kind: 'seek' }; capture(event); seekTo(at.x, true); return; }
    const mark = hitMark(marks, at);
    if (!mark) { setChosen(null); return; }
    setChosen({ node: mark.node, frame: mark.frame });
    // The nodes whose keyframe a drag of this mark moves: a locked node keeps its keyframes.
    const nodes = nodesFor(shownGrade, tracksOn(mark.node), 'all', null).filter(node => !isLocked(node) && moveRange(node, mark.frame) !== null);
    let [low, high] = frames;
    for (const node of nodes) {
      const within = moveRange(node, mark.frame);
      if (within) { low = Math.max(low, within[0]); high = Math.min(high, within[1]); }
    }
    drag.current = { kind: 'mark', mark, nodes, low, high, start: at.x, moved: false, to: mark.frame };
    capture(event);
  };
  const move = (event: PointerEvent<SVGSVGElement>) => {
    const d = drag.current;
    if (!d) return;
    const at = pointOf(event);
    if (d.kind === 'seek') { seekTo(at.x, true); return; }
    if (!d.moved && Math.abs(at.x - d.start) < SLOP_PX) return;
    d.moved = true;
    const of = target;
    if (of === null || d.nodes.length === 0 || d.low > d.high) return;
    const to = Math.min(d.high, Math.max(d.low, frameAtX(frames, width, at.x)));
    if (to === d.to) return;
    const from = d.mark.frame;
    if (!dragging.current) {
      // What the gesture puts back where it is taken back: the settings as they are before its first move.
      if (!gesture.start(d.nodes.flatMap(node => moveKeyframe(of, node, from, from)))) return;
      dragging.current = true;
    }
    d.to = to;
    setLive({ nodes: d.nodes, from, to });
    gesture.preview(d.nodes.flatMap(node => moveKeyframe(of, node, from, to)));
  };
  const up = (event: PointerEvent<SVGSVGElement>) => {
    const d = drag.current;
    if (!d) return;
    drag.current = null;
    if (d.kind === 'seek') { seekTo(pointOf(event).x, false); return; }
    setLive(null);
    // The end of a drag: its last move is what stays, and the keyframe is chosen where it was put.
    if (dragging.current) { dragging.current = false; gesture.end(); setChosen({ node: d.mark.node, frame: d.to }); return; }
    if (!d.moved) onSeek(timelineFrameOf(shownSpan, d.mark.frame), false);
  };
  const cancel = () => {
    const d = drag.current;
    drag.current = null;
    if (dragging.current) { dragging.current = false; gesture.cancel(); }
    if (d?.kind === 'mark') setLive(null);
  };
  const keyDown = (event: KeyboardEvent) => {
    if (event.key !== 'Escape' || !drag.current) return;
    event.preventDefault();
    event.stopPropagation();
    cancel();
  };

  const markItems = (mark: Mark): MenuItem[] => {
    const on = tracksOn(mark.node);
    const eases = nodesFor(shownGrade, on, 'all', null).map(node => easeOf(node, mark.frame)).find(each => each !== null) ?? null;
    return [
      ...(mark.kind === 'static' || mark.mixed ? [{ id: 'kf-to-dynamic', label: labels.toDynamic, run: () => keys.change(on, mark.frame, 'dynamic') }] : []),
      ...(mark.kind === 'dynamic' ? [{ id: 'kf-to-static', label: labels.toStatic, run: () => keys.change(on, mark.frame, 'static') }] : []),
      { id: 'kf-attributes', label: labels.attributes, disabled: eases === null,
        run: () => { if (eases) setEasing({ on, frame: mark.frame, start: eases.start, end: eases.end }); } },
      { id: 'kf-delete', label: labels.deleteKey, keys: 'Alt+]', run: () => keys.remove(on, mark.frame) },
    ];
  };
  const context = (event: MouseEvent<SVGSVGElement>) => {
    event.preventDefault();
    if (target === null) return;
    const at = pointOf(event);
    const mark = hitMark(marks, at);
    if (mark) {
      setChosen({ node: mark.node, frame: mark.frame });
      setMenu({ x: event.clientX, y: event.clientY, label: labels.markMenu, items: markItems(mark) });
      return;
    }
    const row = rowAt(at.y, tracks.length);
    if (row === null) return;
    const on = tracksOn(tracks[row].node);
    setMenu({ x: event.clientX, y: event.clientY, label: labels.trackMenu, items: [
      { id: 'kf-add-dynamic', label: labels.addDynamic, keys: 'Ctrl+[', disabled: playhead === null, run: () => keys.add('dynamic', on) },
      { id: 'kf-add-static', label: labels.addStatic, keys: 'Ctrl+]', disabled: playhead === null, run: () => keys.add('static', on) },
      { id: 'kf-delete-all', label: labels.deleteAll, disabled: tracks[row].keyframes.length === 0, run: () => keys.removeAll(on) },
    ] });
  };

  return <section ref={root} tabIndex={-1} className="color-kf" aria-label={labels.title} data-test="keyframes-panel" onKeyDown={keyDown}>
    <div className="color-kf-bar" role="toolbar" aria-label={labels.tools}>
      <select className="field color-kf-scope" aria-label={labels.scope} value={keys.scope} data-test="kf-scope"
        onChange={(event) => { keys.setScope(event.target.value as KeyScope); event.currentTarget.blur(); }}>
        {KEY_SCOPES.map(each => <option key={each} value={each}>{labels.scopes[each]}</option>)}
      </select>
      <button type="button" className="icon-btn" disabled={!can} aria-label={labels.addDynamic} title={`${labels.addDynamic} (Ctrl+[)`}
        onClick={() => keys.add('dynamic')} data-test="kf-add-dynamic"><AddGlyph kind="dynamic" /></button>
      <button type="button" className="icon-btn" disabled={!can} aria-label={labels.addStatic} title={`${labels.addStatic} (Ctrl+])`}
        onClick={() => keys.add('static')} data-test="kf-add-static"><AddGlyph kind="static" /></button>
      <button type="button" className="icon-btn" disabled={!can || !here} aria-label={labels.deleteKey} title={`${labels.deleteKey} (Alt+])`}
        onClick={() => keys.remove()} data-test="kf-delete"><Trash2 size={13} /></button>
      <button type="button" className="icon-btn" disabled={target === null || !any} aria-label={labels.deleteAll} title={labels.deleteAll}
        onClick={() => keys.removeAll()} data-test="kf-delete-all"><Eraser size={13} /></button>
      <span className="color-kf-gap" />
      <button type="button" className="icon-btn" disabled={frame === null || !any} aria-label={labels.previous} title={`${labels.previous} ([)`}
        onClick={() => keys.step(-1)} data-test="kf-previous"><StepBack size={13} /></button>
      <button type="button" className="icon-btn" disabled={frame === null || !any} aria-label={labels.next} title={`${labels.next} (])`}
        onClick={() => keys.step(1)} data-test="kf-next"><StepForward size={13} /></button>
    </div>
    <div className="color-kf-body">
      <div className="color-kf-heads">
        <div className="color-kf-head color-kf-corner" style={{ height: LANE.ruler }}>
          {playhead !== null && <output className="mono" data-test="kf-frame">{playhead}</output>}
        </div>
        <div className="color-kf-head" data-track="master" style={{ height: LANE.row }}><span className="color-kf-name">{labels.master}</span></div>
        {shownGrade.nodes.map((node, index) => {
          const name = node.label || page.kinds[node.kind];
          const locked = isLocked(node);
          const keyable = NODE_KEYED[node.kind].length > 0;
          return <div key={node.id} className="color-kf-head" data-track={node.id} data-current={node.id === current || undefined} style={{ height: LANE.row }}>
            <button type="button" className="color-kf-name" aria-pressed={node.id === current} title={name} onClick={() => onCurrent(node.id)}
              data-test="kf-track-name">{`${numberText(index + 1)} ${name}`}</button>
            <button type="button" className="icon-btn" aria-pressed={node.enabled} aria-label={labels.enable} title={labels.enable} disabled={target === null}
              onClick={() => { if (target !== null) editing.run(toggleNodes(target, shownGrade, [node.id])); }} data-test="kf-enable">
              {node.enabled ? <Eye size={12} /> : <EyeOff size={12} />}</button>
            <button type="button" className="icon-btn" aria-pressed={locked} aria-label={page.nodeMenu.lock} title={page.nodeMenu.lock} disabled={target === null}
              onClick={() => { if (target !== null) editing.run(toggleLock(target, shownGrade, node.id)); }} data-test="kf-lock">
              {locked ? <Lock size={12} /> : <Unlock size={12} />}</button>
            <button type="button" className="icon-btn color-kf-auto" aria-pressed={keys.auto.has(node.id)} aria-label={labels.autoKey} title={labels.autoKey}
              disabled={!keyable} onClick={() => keys.toggleAuto(node.id)} data-test="kf-auto">A</button>
          </div>;
        })}
      </div>
      <svg ref={svg} className="color-kf-lanes" width="100%" height={height} role="group" aria-label={labels.lanes} data-test="kf-lanes"
        onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={cancel} onContextMenu={context}>
        <rect className="color-kf-ruler" x={0} y={0} width={width} height={LANE.ruler} />
        {ticks.map((tick) => {
          const x = n(xOfFrame(frames, width, tick));
          return <g key={tick} className="color-kf-tick">
            <line x1={x} x2={x} y1={LANE.ruler - 5} y2={LANE.ruler} />
            <text x={x + 3} y={LANE.ruler - 8}>{tick}</text>
          </g>;
        })}
        {tracks.map((track, row) => <rect key={row} className="color-kf-row" x={0} y={rowTop(row)} width={width} height={LANE.row} data-row={row}
          data-master={track.node === null || undefined} data-current={(track.node !== null && track.node === current) || undefined} />)}
        <KeyMarks marks={marks} chosen={chosen} dynamic={labels.dynamic} held={labels.static} />
        {playhead !== null && <line className="color-kf-playhead" data-frame={playhead} x1={n(xOfFrame(frames, width, playhead))} x2={n(xOfFrame(frames, width, playhead))}
          y1={0} y2={height} data-test="kf-playhead" />}
      </svg>
    </div>
    {menu && <ColorMenu menu={menu} onClose={closeMenu} />}
    {easing && <DynamicAttributes start={easing.start} end={easing.end} onClose={() => setEasing(null)}
      onApply={(start, end) => { keys.ease(easing.on, easing.frame, start, end); setEasing(null); }} />}
  </section>;
}
