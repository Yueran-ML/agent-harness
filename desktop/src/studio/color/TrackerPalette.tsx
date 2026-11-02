import { useCallback, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react';
import { useWords } from '../../app/locale';
import { callEngine, type PowerWindow, type TrackComponents, type TrackDirection, type TrackEdit } from '../engine';
import { GRAPH_COLUMNS, GRAPH_CURVES, graph, keyAt, selected as selectedCurves, withCrosshair, withoutCrosshair, type Selection } from './tracker';
import type { EditedNode } from './useNodeEdit';
import type { Tracking } from './useTracking';
import './tracker.css';

const VIEW = { width: GRAPH_COLUMNS, height: 120 };
const BUTTONS: readonly { direction: TrackDirection | 'stop'; glyph: string; word: 'trackOneReverse' | 'trackReverse' | 'stop' | 'trackForward' | 'trackForwardBack' | 'trackOneForward' }[] = [
  { direction: 'one_reverse', glyph: '◀|', word: 'trackOneReverse' },
  { direction: 'reverse', glyph: '◀', word: 'trackReverse' },
  { direction: 'stop', glyph: '■', word: 'stop' },
  { direction: 'forward', glyph: '▶', word: 'trackForward' },
  { direction: 'forward_and_back', glyph: '◀▶', word: 'trackForwardBack' },
  { direction: 'one_forward', glyph: '|▶', word: 'trackOneForward' },
];
const BOXES: readonly (keyof TrackComponents)[] = ['pan', 'tilt', 'zoom', 'rotate', 'perspective'];

/** The window last copied (Copy Track Data, p. 3233), kept while the page is open: pasted within its node. */
let copied: { target: string; node: string; window: string } | null = null;

/**
 * The Tracker palette in Window mode (Reference Manual 20, chapter 140, pp. 3229–3255; PROTOCOL.md,
 * "Window tracking"), for the window selected in the Window palette:
 *
 * - The analyze buttons, in the manual's order (p. 3236): Track One Frame Reverse, Track Reverse,
 *   Stop, Track Forward, Track Forward and Back, Track One Frame Forward. The keys of the
 *   Color › Tracker menu (p. 3231) and the track going on are the page's ({@link Tracking}): Stop works after another
 *   palette was opened, and the palette shows the track again when it is opened.
 *   The engine tracks off the page and writes the window as it goes: the viewer is taken to each
 *   frame it reaches, so the window is seen following, and the whole track is one step to undo.
 * - The five boxes of the transforms to track, set before the track (p. 3235).
 * - Clip and Frame (p. 3236), kept with the window.
 * - The tracker graph, a curve for each transform in the colour of its box (p. 3236), with the
 *   playhead; a box drawn over it selects parts of the curves, and a click clears it (p. 3237).
 * - The Cloud or Point tracker (p. 3234); the point tracker's Add Tracker and Delete Tracker (p. 3238).
 *   Its crosshairs are placed here by number; dragging them on the viewer is still to come.
 * - The Options menu (p. 3238): Reset Track Data on Active Window, Clear Selected Track Data, Add and
 *   Delete Keyframe (Frame mode), Copy and Paste Track Data.
 */
export function TrackerPalette({ edited, window: id, run }: {
  edited: EditedNode | null;
  /** The window selected in the Window palette. */
  window: string | null;
  /** The page's tracking: the track going on, the boxes, the kind of tracker and the keys of the Tracker menu. */
  run: Tracking;
}) {
  const words = useWords();
  const said = words.label.studio.colorPage.trackerPalette;
  const hints = words.hint.studio.colorPage.tracker;
  const node = edited?.node.kind === 'corrector' ? edited.node : null;
  const window: PowerWindow | null = node?.params.windows?.find(each => each.id === id) ?? null;
  const tracking = window?.tracking;
  const { components, setComponents, tracker, setTracker, running, status, setStatus, start, stop, refused } = run;
  const [scale, setScale] = useState(1);
  const [box, setBox] = useState<Selection | null>(null);
  const [crosshair, setCrosshair] = useState(0);
  const drawn = useMemo(() => graph(tracking?.data, edited?.frame ?? 0, scale), [tracking?.data, edited?.frame, scale]);
  const crosshairs = tracking?.crosshairs ?? [];
  // The crosshair chosen, while the one just added is not in the project yet: the last there is.
  const pick = Math.max(0, Math.min(crosshair, crosshairs.length - 1));

  const now = useRef({ edited, window });
  now.current = { edited, window };

  const edit = useCallback((change: TrackEdit, target: PowerWindow | null = now.current.window) => {
    const context = now.current.edited;
    if (context === null || target === null || context.node.kind !== 'corrector') return;
    void callEngine(context.bridge, 'track.edit', { grade: context.target, node: context.node.id, window: target.id, revision: context.revision, edit: change },
      context.session === undefined ? undefined : { expectedSession: context.session }).catch(refused);
  }, [refused]);

  // A box drawn over the graph, in frames and its own y.
  const svg = useRef<SVGSVGElement>(null);
  const drag = useRef<{ x: number; y: number } | null>(null);
  const at = (event: ReactPointerEvent): { frame: number; y: number } => {
    const rect = svg.current?.getBoundingClientRect();
    const width = rect && rect.width > 0 ? rect.width : VIEW.width;
    const height = rect && rect.height > 0 ? rect.height : VIEW.height;
    const x = (event.clientX - (rect?.left ?? 0)) / width;
    const y = (event.clientY - (rect?.top ?? 0)) / height;
    return { frame: drawn.from + x * (drawn.to - drawn.from), y: y * 2.4 - 1.2 };
  };
  const toX = (frame: number) => ((frame - drawn.from) / (drawn.to - drawn.from)) * VIEW.width;
  const toY = (y: number) => ((y + 1.2) / 2.4) * VIEW.height;
  const chosen = box === null ? null : selectedCurves(drawn.lines, box);

  if (node === null) return <p className="caption color-empty" data-test="no-corrector">{words.hint.studio.colorPage.noCorrector}</p>;
  if (window === null) return <p className="caption color-empty" data-test="tracker-no-window">{hints.noWindow}</p>;
  const frame = edited?.frame ?? 0;
  const mode = tracking?.mode ?? 'clip';
  return <div className="tracker-palette" data-test="tracker-palette">
    <div className="tracker-row">
      <div className="tracker-boxes" role="group" aria-label={said.transforms}>
        {BOXES.map(each => <label key={each} className="tracker-box" data-box={each}>
          <input type="checkbox" checked={components[each]} data-test={`tracker-box-${each}`}
            onChange={event => { const on = event.currentTarget.checked; setComponents(old => ({ ...old, [each]: on })); }} />
          <span style={{ color: GRAPH_CURVES.find(curve => curve.box === each)?.color }}>{said[each]}</span>
        </label>)}
      </div>
      <div className="tracker-analyze" role="group" aria-label={said.analyze}>
        {BUTTONS.map(each => <button key={each.direction} type="button" className="icon-btn" data-test={`tracker-${each.direction}`}
          aria-label={said[each.word]} title={said[each.word]}
          disabled={each.direction === 'stop' ? running === null : running !== null}
          onClick={() => (each.direction === 'stop' ? stop() : start(each.direction))}>{each.glyph}</button>)}
      </div>
      <div className="tracker-mode" role="group" aria-label={said.mode}>
        {(['clip', 'frame'] as const).map(each => <button key={each} type="button" aria-pressed={mode === each} data-test={`tracker-mode-${each}`}
          onClick={() => { if (mode !== each) edit({ kind: 'mode', mode: each }); }}>{said[each]}</button>)}
      </div>
    </div>
    <div className="tracker-graph-row">
      <svg ref={svg} className="tracker-graph" viewBox={`0 0 ${VIEW.width} ${VIEW.height}`} preserveAspectRatio="none" role="img"
        aria-label={said.graph} data-test="tracker-graph"
        onPointerDown={event => { const p = at(event); drag.current = { x: p.frame, y: p.y }; setBox(null); event.currentTarget.setPointerCapture?.(event.pointerId); }}
        onPointerMove={event => { if (drag.current === null) return; const p = at(event); setBox({ from: drag.current.x, to: p.frame, top: drag.current.y, bottom: p.y }); }}
        onPointerUp={() => { drag.current = null; setBox(old => (old !== null && Math.abs(old.to - old.from) < 0.5 ? null : old)); }}>
        <line x1={0} x2={VIEW.width} y1={toY(0)} y2={toY(0)} className="tracker-axis" />
        {drawn.lines.map(line => <polyline key={line.component} data-curve={line.component} fill="none" stroke={line.color} strokeWidth={1.2}
          vectorEffect="non-scaling-stroke" points={line.points.map(([f, y]) => `${toX(f).toFixed(2)},${toY(y).toFixed(2)}`).join(' ')} />)}
        {Object.keys(tracking?.data.keys ?? {}).map(Number).filter(Number.isFinite).map(key => <rect key={key} data-test="tracker-key"
          x={toX(key) - 3} y={VIEW.height - 7} width={6} height={6} className="tracker-key" />)}
        <line x1={toX(frame)} x2={toX(frame)} y1={0} y2={VIEW.height} className="tracker-playhead" data-test="tracker-playhead" />
        {box !== null && <rect data-test="tracker-selection" className="tracker-selection" x={toX(Math.min(box.from, box.to))} width={Math.abs(toX(box.to) - toX(box.from))}
          y={toY(Math.min(box.top, box.bottom))} height={Math.abs(toY(box.bottom) - toY(box.top))} />}
      </svg>
      <input type="range" className="tracker-scale" min={0.25} max={4} step={0.25} value={scale} aria-label={said.scale}
        onChange={event => setScale(Number(event.currentTarget.value))} />
    </div>
    <div className="tracker-row">
      <select aria-label={said.tracker} value={tracker} data-test="tracker-kind" onChange={event => setTracker(event.currentTarget.value as 'cloud' | 'point')}>
        <option value="cloud">{said.cloud}</option>
        <option value="point">{said.point}</option>
      </select>
      {tracker === 'point' && <div className="tracker-points" role="group" aria-label={said.point}>
        <button type="button" data-test="tracker-add-point" onClick={() => { edit({ kind: 'crosshairs', points: withCrosshair(crosshairs) }); setCrosshair(crosshairs.length); }}>{said.addTracker}</button>
        <button type="button" data-test="tracker-delete-point" disabled={crosshairs.length === 0}
          onClick={() => { edit({ kind: 'crosshairs', points: withoutCrosshair(crosshairs, pick) }); setCrosshair(0); }}>{said.deleteTracker}</button>
        {crosshairs.map((point, i) => <span key={i} className="tracker-crosshair" data-selected={i === pick || undefined}>
          <button type="button" aria-pressed={i === pick} onClick={() => setCrosshair(i)}>{said.crosshair(i + 1)}</button>
          {i === pick && (['x', 'y'] as const).map((axis, a) => <label key={axis}>{said[axis]}
            <input type="number" step={0.1} min={0} max={100} value={Number((point[a] * 100).toFixed(2))} data-test={`tracker-point-${axis}`}
              onChange={event => {
                const value = Number(event.currentTarget.value) / 100;
                if (!Number.isFinite(value)) return;
                edit({ kind: 'crosshairs', points: crosshairs.map((p, j) => (j === i ? (a === 0 ? [value, p[1]] : [p[0], value]) : p)) });
              }} /></label>)}
        </span>)}
      </div>}
      <select aria-label={said.options} value="" data-test="tracker-options" onChange={event => {
        const asked = event.currentTarget.value;
        event.currentTarget.value = '';
        if (asked === 'reset') edit({ kind: 'reset' });
        else if (asked === 'clear' && chosen !== null) { edit({ kind: 'clear', ...chosen }); setBox(null); }
        else if (asked === 'add_key') edit({ kind: 'add_key', frame });
        else if (asked === 'delete_key') edit({ kind: 'delete_key', frame });
        else if (asked === 'copy' && edited !== null) { copied = { target: JSON.stringify(edited.target), node: edited.node.id, window: window.id }; setStatus(hints.copied); }
        else if (asked === 'paste' && copied !== null) edit({ kind: 'paste', from: copied.window });
      }}>
        <option value="" disabled>{said.options}</option>
        <option value="reset">{said.reset}</option>
        <option value="clear" disabled={chosen === null}>{said.clearSelected}</option>
        <option value="add_key" disabled={mode !== 'frame' || keyAt(tracking?.data, frame)}>{said.addKey}</option>
        <option value="delete_key" disabled={!keyAt(tracking?.data, frame)}>{said.deleteKey}</option>
        <option value="copy">{said.copy}</option>
        <option value="paste" disabled={copied === null || edited === null || copied.node !== edited.node.id
          || copied.target !== JSON.stringify(edited.target) || copied.window === window.id}>{said.paste}</option>
      </select>
    </div>
    {status !== null && <p className="caption tracker-status" role="status" data-test="tracker-status">{status}</p>}
  </div>;
}
