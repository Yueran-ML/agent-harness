import { memo, useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from 'react';
import { Blend, Ellipsis, ImagePlus, Move, Square, X } from 'lucide-react';
import { keyOf } from '../../app/keys';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { EngineError, type Clip, type Composition, type FusionInput, type FusionKind, type FusionLink, type FusionNode, type Op } from '../engine';
import { editingAllowed, inStudio } from '../keys';
import type { Menu, MenuItem } from '../color/ColorMenu';
import { CoveringMenu } from '../color/CoveringMenu';
import { linkHalves, linkPath, NODE_HEIGHT, NODE_WIDTH, shownLabel, type Placed, type Point } from '../color/graph';
import { KeyedMark, ZOOM_LEAST, ZOOM_MOST } from '../color/NodeEditor';
import type { GradeEditing } from '../color/useGradeEditing';
import { refusalText } from '../inspector/refusal';
import { useAudioGesture } from '../timeline/useAudioGesture';
import {
  ADDABLE, addedNode, addNode, connect, disconnect, fusionKey, isFirst, moveNodes, reconnect, relabel, removeComposition, removeNodes, resetComposition,
  restoreNodes, togglePass, withFirst, type FusionAt, type FusionCommand,
} from './edits';
import {
  hasOutput, INPUTS, layoutComposition, linkEnds, linkName, namesOf, nodesIn, placesOf, portAt, portName, portPoint,
  type FusionLayout, type FusionPort, type PlacedFusion,
} from './graph';
import { hasKeys } from './settings';
import '../color/color.css';
import '../color/nodes.css';
import './fusion.css';

/** How far the pointer goes before a press becomes a drag. */
const DRAG_PX = 3;
/** How long nodes that were dropped stand where they were dropped, at most, until the engine tells of them. */
const HOLD_MS = 1500;
const ZOOM_STEP = 1.25;
const NO_PLACES: Placed = {};
/** The face of the button that adds a node of each kind. */
const ADD_ICONS = { merge: Blend, transform: Move, background: Square, media_in: ImagePlus } as const;

/** The nodes that are chosen: the one chosen last, whose settings the Inspector shows where it is the only one, and the others. */
export interface FusionSelection { current: string | null; others: readonly string[] }
export const NO_SELECTION: FusionSelection = { current: null, others: [] };

/** Why the engine refused, where it is a refusal that changes nothing: a node dropped where it was, which is no failure. */
const isEmpty = (error: unknown): boolean => error instanceof EngineError && typeof error.data === 'object' && error.data !== null
  && (error.data as { reason?: unknown }).reason === 'empty';

/**
 * A node of a composition: a card with a stripe in the colour of its kind and its name, its inputs
 * down its left side (a Merge's background orange above its foreground green, as in Fusion,
 * p. 2162) and its output at the middle of its right side. A node that is passed through is drawn
 * dimmed, and one whose settings have keys has a badge. Drawn again only when what it shows changes:
 * while one node is dragged the others are left as they are.
 */
const NodeCard = memo(function NodeCard({ node, x, y, name, kind, current, chosen, lit, keyed, passed, ports, onLabel }: {
  node: FusionNode; x: number; y: number; name: string; kind: string; current: boolean; chosen: boolean;
  /** What each port says of itself where the pointer rests on it: the names of the inputs, and of the output. */
  ports: Record<FusionInput | 'output', string>;
  /** The port that a link being drawn would end on, as `portName` names it, or null. */
  lit: string | null;
  /** What the badge of keys says, where the node has keys; and what a node passed through says of itself. */
  keyed: string | null; passed: string | null;
  onLabel: (node: string) => void;
}) {
  const inputs = INPUTS[node.kind];
  return <g className="color-node fusion-node" data-node={node.id} data-kind={node.kind} data-current={current || undefined} data-chosen={chosen || undefined}
    data-pass-through={node.pass_through || undefined}>
    <title>{[name, kind, passed ?? '', keyed ?? ''].filter(Boolean).join(' · ')}</title>
    <rect className="color-node-card" x={x} y={y} width={NODE_WIDTH} height={NODE_HEIGHT} rx={5} />
    <rect className="fusion-node-stripe" x={x + 1} y={y + 1} width={NODE_WIDTH - 2} height={6} rx={3} />
    <text className="fusion-node-name" x={x + NODE_WIDTH / 2} y={y + NODE_HEIGHT / 2 + 5} textAnchor="middle" data-test="node-label"
      onDoubleClick={() => onLabel(node.id)}>{shownLabel(name)}</text>
    {keyed !== null && <KeyedMark x={x + NODE_WIDTH / 2} y={y + NODE_HEIGHT - 8} title={keyed} />}
    {inputs.map((input, at) => {
      const py = y + (NODE_HEIGHT * (at + 1)) / (inputs.length + 1);
      const port = portName({ side: 'in', input });
      return <path key={input} className="color-port fusion-port" data-input={input} data-port={port} data-lit={lit === port || undefined}
        d={`M ${x - 1} ${py - 5} L ${x + 6} ${py} L ${x - 1} ${py + 5} Z`}><title>{ports[input]}</title></path>;
    })}
    {hasOutput(node.kind) && <rect className="color-port fusion-port" data-port="out" data-lit={lit === 'out' || undefined}
      x={x + NODE_WIDTH - 5} y={y + NODE_HEIGHT / 2 - 4} width={7} height={8}><title>{ports.output}</title></rect>}
  </g>;
});

/**
 * A link of a composition, from an output to an input, with two halves to take hold of: either
 * chooses it when it is clicked, and the second takes its end along when it is dragged. Drawn again
 * only when one of its ends moves or what is said of it changes.
 */
const LinkLine = memo(function LinkLine({ name, input, fromX, fromY, toX, toY, chosen, replacing, hover }: {
  name: string; input: FusionInput; fromX: number; fromY: number; toX: number; toY: number; chosen: boolean; replacing: boolean; hover: 'start' | 'end' | null;
}) {
  const from = { x: fromX, y: fromY };
  const to = { x: toX, y: toY };
  const [first, second] = linkHalves(from, to);
  return <g>
    <path className="color-link fusion-link" data-link={name} data-input={input} d={linkPath(from, to)} data-chosen={chosen || undefined}
      data-replacing={replacing || undefined} data-hover={hover ?? undefined} />
    <path className="color-link-hit" data-link-hit={name} data-half="start" d={first} />
    <path className="color-link-hit" data-link-hit={name} data-half="end" d={second} />
  </g>;
});

/** A press in the graph, and what it becomes as the pointer moves. */
type Drag =
  /** Nodes dragged: `live` once the gesture that sends their places has begun. */
  | { kind: 'node'; id: string; ids: string[]; start: Point; moved: boolean; live: boolean; ctrl: boolean; shift: boolean; base: FusionLayout; last: Placed }
  /** A link drawn from a port; `detached` is the link whose end was taken up, which goes with it. */
  | { kind: 'wire'; from: FusionPort; start: Point; moved: boolean; detached: FusionLink | null; base: FusionLayout }
  | { kind: 'half'; link: FusionLink; half: 'start' | 'end'; start: Point; moved: boolean; base: FusionLayout }
  | { kind: 'marquee'; start: Point; moved: boolean; additive: boolean; base: FusionLayout }
  | { kind: 'pan'; x: number; y: number; left: number; top: number };

/** What a drag is about to do, drawn until it ends. */
interface Preview {
  moved?: Placed;
  wire?: { from: Point; to: Point } | null;
  port?: FusionPort | null;
  replacing?: string | null;
  detached?: string | null;
  marquee?: { a: Point; b: Point } | null;
}

const linkInto = (layout: FusionLayout, port: FusionPort) => layout.links.find(link => link.to === port.node && link.input === port.input) ?? null;

/**
 * The node editor of the Fusion page (DaVinci Resolve Reference Manual 20, Ch. 67, pp. 1316–1365):
 * the composition of the current clip, drawn and edited with what the Color page's node editor is
 * made of (its measures, the curve and the halves of a link, its style sheet, its menus and the way
 * it sends a change and says a refusal), about the nodes and the named inputs of a composition:
 *
 * - a node added from the tool bar or from the right-click menu (Merge, Transform, Background,
 *   MediaIn), where the menu was opened; with one node chosen, a node that has an input goes in
 *   after it (`addNode`), and the new node is the one chosen;
 * - a node chosen with a click, more with Ctrl+click or Shift+click or a box drawn round them, all
 *   with Ctrl+A; moved by dragging, which is one gesture: each move sends the places whole, the
 *   release ends it, and the whole is one step to undo (Escape takes it back);
 * - a link drawn from an output to an input, or from an input to an output; one into an input that
 *   has a link takes its place; the end of a link dragged to another input, or off to take the link
 *   away; a link chosen with a click and deleted with Delete or Backspace;
 * - Delete deletes the nodes chosen, joining what fed each to what it fed where that is a single
 *   chain (`removeNodes`); the MediaOut stays; Ctrl+P passes the nodes chosen through; F2, a double
 *   click on its name, or its menu renames a node;
 * - Reset Fusion Composition (p. 1272) and Remove Composition, from the menu of the background and
 *   of the button at the right of the tool bar.
 *
 * A clip without a composition is shown with the first one (`shownComposition`), and says so: the
 * first change sends it before itself (`withFirst`). Where a node lies is the project's (`pos`).
 * How far the editor is zoomed and what is chosen are the view's. What the engine refuses is said
 * above the graph, as the Color page says it; a change that changes nothing (`empty`, a node
 * dragged back to where it was) is no refusal and is not said.
 */
export function FusionNodeEditor({ bridge, session, revision, clip, composition, at, selection, onSelection, focused, editing }: {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number;
  /** The clip whose composition is shown: it has none yet where `fusion` is not there. */
  clip: Clip;
  /** The composition as it is shown: the clip's, or the first one. */
  composition: Composition;
  at: FusionAt;
  selection: FusionSelection;
  onSelection: (selection: FusionSelection) => void;
  /** Whether the node editor has the focus of the page: Delete, Ctrl+P, F2 and Ctrl+A are its then. */
  focused: boolean;
  editing: GradeEditing;
}) {
  const words = useWords();
  const labels = words.label.studio.fusionPage;
  const colorLabels = words.label.studio.colorPage;
  const menuWords = colorLabels.nodeMenu;
  const has = clip.fusion !== undefined;
  const canvas = useRef<HTMLDivElement>(null);
  const [zoom, setZoomState] = useState(1);
  const [picked, setPicked] = useState<string | null>(null);
  const [hover, setHover] = useState<{ link: string; half: 'start' | 'end' } | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [menu, setMenu] = useState<Menu | null>(null);
  const [labelling, setLabelling] = useState<{ id: string; value: string } | null>(null);
  // Nodes dropped where the engine has not yet told of them stand where they were dropped, until it does or a while has passed.
  const [held, setHeld] = useState<Placed | null>(null);
  const drag = useRef<Drag | null>(null);

  const names = useMemo(() => namesOf(composition, labels.kinds), [composition, labels.kinds]);
  const portNames = useMemo(() => ({ ...labels.inputs, output: labels.output }), [labels.inputs, labels.output]);
  const kept = useMemo(() => placesOf(composition), [composition]);
  const placed = useMemo(() => (held === null ? kept : { ...kept, ...held }), [kept, held]);
  const base = useMemo(() => layoutComposition(composition, placed), [composition, placed]);
  const layout = useMemo(() => (preview?.moved ? layoutComposition(composition, { ...placed, ...preview.moved }) : base), [composition, base, placed, preview?.moved]);
  const selected = useMemo(() => composition.nodes.filter(node => node.id === selection.current || selection.others.includes(node.id)).map(node => node.id),
    [composition, selection]);
  const pickedLink = picked !== null && composition.links.some(link => linkName(link) === picked) ? picked : null;

  // What the handlers of keys and of the pointer read, as it is now.
  const now = useRef({ composition, clip, has, at, selection, selected, pickedLink, focused, editing, onSelection, zoom, menu, labelling, base, words });
  now.current = { composition, clip, has, at, selection, selected, pickedLink, focused, editing, onSelection, zoom, menu, labelling, base, words };

  // The places of nodes dragged are sent as one gesture, so that a drag is one step to undo.
  const gesture = useAudioGesture({
    bridge, session, revision, identity: `${at.timeline}/${at.clip}/fusion-nodes`, locked: false,
    onFailure: (error) => {
      setHeld(null);
      if (!isEmpty(error)) now.current.editing.setProblem(refusalText(error, now.current.words.hint.studio));
    },
  });
  const gestureNow = useRef(gesture);
  gestureNow.current = gesture;

  // Nodes that were dropped are let go once the project has them where they were dropped, or after a while.
  useEffect(() => {
    if (held === null) return undefined;
    const there = Object.entries(held).every(([id, at]) => {
      const pos = composition.nodes.find(node => node.id === id)?.pos;
      return pos != null && pos.x === Math.round(at.x) && pos.y === Math.round(at.y);
    });
    if (there) { setHeld(null); return undefined; }
    const timer = setTimeout(() => setHeld(null), HOLD_MS);
    return () => clearTimeout(timer);
  }, [held, composition]);

  const setZoom = useCallback((next: number, around?: { client: Point; graph: Point }) => {
    const value = Math.min(ZOOM_MOST, Math.max(ZOOM_LEAST, next));
    setZoomState(value);
    const element = canvas.current;
    if (element && around) {
      const box = element.getBoundingClientRect();
      element.scrollLeft = around.graph.x * value - (around.client.x - box.left);
      element.scrollTop = around.graph.y * value - (around.client.y - box.top);
    }
  }, []);

  /** Send a change as one transaction, the first composition before it where the clip has none; `then` is told what the engine did. */
  const run = useCallback((ops: Op[], then?: (done: readonly Op[]) => void) => {
    const { editing: sending, at: where, has: there } = now.current;
    const sent = withFirst(where, there, ops);
    if (sent.length > 0) sending.run(sent, applied => then?.(applied.done));
  }, []);

  /** Add a node of a kind, after the one node chosen where it can go there, at `pos` or where the editor lays it out: it is the one chosen then. */
  const add = useCallback((kind: FusionKind, pos: Point | null) => {
    const { composition: of, at: where, selection: chosen } = now.current;
    const after = chosen.current !== null && chosen.others.length === 0 ? chosen.current : null;
    run(addNode(where, of, kind, after, pos), (done) => {
      const made = addedNode(done);
      if (made !== null) now.current.onSelection({ current: made, others: [] });
    });
  }, [run]);

  const startLabel = useCallback((id: string) => {
    const node = now.current.composition.nodes.find(each => each.id === id);
    if (node) setLabelling({ id, value: node.label });
  }, []);
  const commitLabel = useCallback(() => {
    const { composition: of, at: where, labelling: writing } = now.current;
    // Enter, and then the blur of the field as it goes, commit once.
    now.current.labelling = null;
    setLabelling(null);
    if (writing) run(relabel(where, of, writing.id, writing.value.trim()));
  }, [run]);

  /** Give a drag up: nodes dragged go back where they were, and what was drawn of it is drawn no longer. */
  const cancelDrag = useCallback(() => {
    const press = drag.current;
    if (!press) return false;
    drag.current = null;
    setPreview(null);
    if (press.kind === 'node' && press.live) gestureNow.current.cancel();
    return true;
  }, []);

  /** Do what a key asks (see `fusionKey`); whether it was the editor's to do. */
  const perform = useCallback((command: FusionCommand): boolean => {
    const state = now.current;
    if (command.kind === 'cancel') {
      if (cancelDrag()) return true;
      if (state.pickedLink !== null) { setPicked(null); return true; }
      return false;
    }
    if (!state.focused) return false;
    switch (command.kind) {
      case 'delete': {
        const link = state.pickedLink === null ? undefined : state.composition.links.find(each => linkName(each) === state.pickedLink);
        if (link) { setPicked(null); run(disconnect(state.at, link)); return true; }
        // Backspace deletes no node, as in the Color page's editor.
        if (command.key === 'Backspace') return false;
        const ops = removeNodes(state.at, state.composition, state.selected);
        if (ops.length === 0) return false;
        run(ops);
        return true;
      }
      case 'passThrough':
        if (state.selected.length === 0) return false;
        run(togglePass(state.at, state.composition, state.selected));
        return true;
      case 'rename':
        if (state.selection.current === null) return false;
        startLabel(state.selection.current);
        return true;
      case 'selectAll': {
        const ids = state.composition.nodes.map(node => node.id);
        state.onSelection({ current: state.selection.current ?? ids[0] ?? null, others: ids.filter(id => id !== (state.selection.current ?? ids[0])) });
        return true;
      }
      default: return false;
    }
  }, [cancelDrag, run, startLabel]);

  // The keys of the node editor, while nothing is written in, no menu is open and no dialog is. The Spline Editor keeps its own keys to itself.
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key) || now.current.menu || now.current.labelling) return;
      const command = fusionKey(key);
      if (command && perform(command)) event.preventDefault();
    };
    window.addEventListener('keydown', down);
    return () => window.removeEventListener('keydown', down);
  }, [perform]);

  // The wheel zooms about the pointer, as in the Color page's editor.
  useEffect(() => {
    const element = canvas.current;
    if (!element) return undefined;
    const wheel = (event: WheelEvent) => {
      if (event.deltaY === 0) return;
      event.preventDefault();
      const box = element.getBoundingClientRect();
      const scale = now.current.zoom;
      const graph = { x: (event.clientX - box.left + element.scrollLeft) / scale, y: (event.clientY - box.top + element.scrollTop) / scale };
      setZoom(scale * (event.deltaY < 0 ? 1.1 : 1 / 1.1), { client: { x: event.clientX, y: event.clientY }, graph });
    };
    element.addEventListener('wheel', wheel, { passive: false });
    return () => element.removeEventListener('wheel', wheel);
  }, [setZoom]);

  const pointOf = (event: { clientX: number; clientY: number }): Point => {
    const element = canvas.current!;
    const box = element.getBoundingClientRect();
    const scale = now.current.zoom;
    return { x: (event.clientX - box.left + element.scrollLeft) / scale, y: (event.clientY - box.top + element.scrollTop) / scale };
  };
  /** The port under an element, or near the point. */
  const portUnder = (element: Element, point: Point, layoutNow: FusionLayout): FusionPort | null => {
    const portElement = element.closest<SVGElement>('[data-port]');
    const nodeElement = element.closest<SVGElement>('[data-node]');
    if (portElement && nodeElement) {
      const name = portElement.dataset.port!;
      return name === 'out' ? { node: nodeElement.dataset.node!, side: 'out', input: null }
        : { node: nodeElement.dataset.node!, side: 'in', input: name.slice(3) as FusionInput };
    }
    return portAt(layoutNow, point);
  };
  /** The port that a link drawn from `from` may end on at a point: one of the other side, of another node. */
  const endAt = (layoutNow: FusionLayout, from: FusionPort, point: Point) => portAt(layoutNow, point, each => each.side !== from.side && each.node !== from.node);

  const pointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    const element = event.target as Element;
    if (element.closest('.color-label-field')) return;
    if (event.button === 1) {
      event.preventDefault();
      drag.current = { kind: 'pan', x: event.clientX, y: event.clientY, left: canvas.current!.scrollLeft, top: canvas.current!.scrollTop };
      canvas.current!.setPointerCapture?.(event.pointerId ?? 1);
      return;
    }
    if (event.button !== 0) return;
    const point = pointOf(event);
    const ctrl = event.ctrlKey || event.metaKey;
    const port = portUnder(element, point, base);
    const hit = element.closest<SVGElement>('[data-link-hit]');
    const nodeElement = element.closest<SVGElement>('[data-node]');
    if (port) {
      // From an output, a new link; from an input that has one, the end of that link, taken up; from an input that has none, a link to an output.
      const into = port.side === 'in' ? linkInto(base, port) : null;
      const from: FusionPort = into ? { node: into.from, side: 'out', input: null } : port;
      drag.current = { kind: 'wire', from, start: point, moved: false, detached: into, base };
    } else if (hit) {
      const link = base.links.find(each => linkName(each) === hit.dataset.linkHit);
      if (link) drag.current = { kind: 'half', link, half: hit.dataset.half as 'start' | 'end', start: point, moved: false, base };
    } else if (nodeElement) {
      const id = nodeElement.dataset.node!;
      const inSelection = selected.includes(id);
      if (!ctrl && !event.shiftKey && id !== selection.current) {
        // A press makes the node the one chosen last; the others chosen stay chosen while it may yet drag them along.
        onSelection({ current: id, others: inSelection ? selected.filter(each => each !== id) : [] });
      }
      const ids = inSelection && !ctrl ? selected : [id];
      drag.current = { kind: 'node', id, ids, start: point, moved: false, live: false, ctrl, shift: event.shiftKey, base, last: NO_PLACES };
    } else {
      drag.current = { kind: 'marquee', start: point, moved: false, additive: ctrl || event.shiftKey, base };
    }
    canvas.current!.setPointerCapture?.(event.pointerId ?? 1);
  };

  const pointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const press = drag.current;
    if (!press) {
      const hit = (event.target as Element).closest<SVGElement>('[data-link-hit]');
      const next = hit ? { link: hit.dataset.linkHit!, half: hit.dataset.half as 'start' | 'end' } : null;
      if (next?.link !== hover?.link || next?.half !== hover?.half) setHover(next);
      return;
    }
    if (press.kind === 'pan') {
      canvas.current!.scrollLeft = press.left - (event.clientX - press.x);
      canvas.current!.scrollTop = press.top - (event.clientY - press.y);
      return;
    }
    const point = pointOf(event);
    const far = Math.hypot(point.x - press.start.x, point.y - press.start.y) * zoom >= DRAG_PX;
    if (!press.moved && !far) return;
    press.moved = true;
    if (press.kind === 'half') {
      // The second half of a link takes its end along; the first does nothing as it is dragged.
      if (press.half === 'start') return;
      drag.current = { kind: 'wire', from: { node: press.link.from, side: 'out', input: null }, start: press.start, moved: true, detached: press.link, base: press.base };
    }
    const going = drag.current!;
    if (going.kind === 'node') {
      const dx = point.x - going.start.x;
      const dy = point.y - going.start.y;
      const moved: Record<string, Point> = {};
      for (const id of going.ids) {
        const was = going.base.nodes.find(each => each.node.id === id);
        if (was) moved[id] = { x: Math.max(0, Math.round(was.x + dx)), y: Math.max(0, Math.round(was.y + dy)) };
      }
      going.last = moved;
      setPreview({ moved });
      // The gesture begins with the first move that it can be sent with; what it puts back is where the nodes were, or no composition where the clip had none.
      const { composition: of, at: where, has: there } = now.current;
      if (!going.live) going.live = gesture.start(there ? restoreNodes(where, of, going.ids) : [{ op: 'set_fusion', ...where, value: null }]);
      if (going.live) gesture.preview(withFirst(where, there, moveNodes(where, of, moved, true)));
    } else if (going.kind === 'wire') {
      const port = endAt(going.base, going.from, point);
      const anchor = portPoint(going.base, going.from) ?? point;
      const taken = port && port.side === 'in' ? linkInto(going.base, port) : null;
      setPreview({
        wire: going.from.side === 'out' ? { from: anchor, to: point } : { from: point, to: anchor }, port,
        replacing: taken && (!going.detached || linkName(taken) !== linkName(going.detached)) ? linkName(taken) : null,
        detached: going.detached ? linkName(going.detached) : null,
      });
    } else if (going.kind === 'marquee') {
      setPreview({ marquee: { a: going.start, b: point } });
    }
  };

  const pointerUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    const press = drag.current;
    drag.current = null;
    setPreview(null);
    if (canvas.current?.hasPointerCapture?.(event.pointerId ?? 1)) canvas.current.releasePointerCapture(event.pointerId ?? 1);
    if (!press || press.kind === 'pan') return;
    const point = pointOf(event);
    const others = selection.others;
    if (press.kind === 'node') {
      if (!press.moved) {
        if (press.ctrl) {
          // Ctrl+click chooses a node more, or one chosen no more; the one chosen last stays chosen.
          if (selection.current === null) onSelection({ current: press.id, others: [] });
          else if (press.id !== selection.current) onSelection({ current: selection.current, others: others.includes(press.id) ? others.filter(each => each !== press.id) : [...others, press.id] });
          return;
        }
        if (press.shift) {
          if (selection.current === null) onSelection({ current: press.id, others: [] });
          else if (press.id !== selection.current && !others.includes(press.id)) onSelection({ current: selection.current, others: [...others, press.id] });
          return;
        }
        onSelection({ current: press.id, others: [] });
        return;
      }
      setHeld(press.last);
      // The release ends the gesture; where none could begin, the places are sent as one transaction, which is one step too.
      if (press.live) gesture.end();
      else run(moveNodes(at, composition, press.last));
      return;
    }
    if (press.kind === 'half') {
      // A link clicked is chosen, for Delete.
      if (!press.moved) setPicked(linkName(press.link));
      return;
    }
    if (press.kind === 'marquee') {
      setPicked(null);
      if (!press.moved) { onSelection(NO_SELECTION); return; }
      const inBox = nodesIn(press.base, press.start, point);
      const all = press.additive ? [...new Set([...selected, ...inBox])] : inBox;
      const current = selection.current !== null && all.includes(selection.current) ? selection.current : all.at(-1) ?? null;
      onSelection({ current, others: all.filter(each => each !== current) });
      return;
    }
    // A link drawn: from an output into an input, or from an input that had no link to an output.
    if (!press.moved) return;
    const port = endAt(press.base, press.from, point);
    if (press.from.side === 'out') {
      if (port && port.input !== null) {
        run(press.detached ? reconnect(at, press.detached, { to: port.node, input: port.input })
          : connect(at, composition, { from: press.from.node, to: port.node, input: port.input }));
        return;
      }
      // The end of a link dragged off into nothing takes the link away.
      if (press.detached) run(disconnect(at, press.detached));
      return;
    }
    if (port && press.from.input !== null) run(connect(at, composition, { from: port.node, to: press.from.node, input: press.from.input }));
  };

  /** The items that add a node of each kind, at a point of the graph or where the editor lays it out. */
  const addItems = (pos: Point | null): MenuItem[] => ADDABLE.map(kind => ({ id: `add-${kind}`, label: labels.kinds[kind], run: () => add(kind, pos) }));
  /** The menu of the background: Add Node ▸, the two commands of the whole composition, and the zoom. */
  const graphItems = (pos: Point | null): MenuItem[] => [
    { id: 'add-node', label: menuWords.addNode, items: addItems(pos) },
    { id: 'reset-composition', label: labels.resetComposition, disabled: !has || isFirst(composition), run: () => run(resetComposition(at, clip)) },
    { id: 'remove-composition', label: labels.removeComposition, disabled: !has, run: () => run(removeComposition(at, clip)) },
    { id: 'zoom-in', label: menuWords.zoomIn, run: () => setZoom(zoom * ZOOM_STEP) },
    { id: 'zoom-out', label: menuWords.zoomOut, run: () => setZoom(zoom / ZOOM_STEP) },
    { id: 'zoom-original', label: menuWords.originalSize, run: () => setZoom(1) },
  ];

  const contextMenu = (event: ReactMouseEvent<HTMLDivElement>) => {
    event.preventDefault();
    const element = event.target as Element;
    const point = pointOf(event);
    const place = { x: event.clientX, y: event.clientY };
    const hit = element.closest<SVGElement>('[data-link-hit], [data-link]');
    const nodeElement = element.closest<SVGElement>('[data-node]');
    if (hit && !nodeElement) {
      const name = hit.dataset.linkHit ?? hit.dataset.link;
      const link = composition.links.find(each => linkName(each) === name);
      if (link) setMenu({ ...place, label: menuWords.link, items: [{ id: 'delete-link', label: menuWords.deleteLink, keys: 'Delete', run: () => run(disconnect(at, link)) }] });
      return;
    }
    if (nodeElement) {
      const id = nodeElement.dataset.node!;
      const node = composition.nodes.find(each => each.id === id);
      if (!node) return;
      // The node right-clicked is the one chosen: a node added from its menu goes in after it.
      if (id !== selection.current || selection.others.length > 0) onSelection({ current: id, others: [] });
      now.current.selection = { current: id, others: [] };
      setMenu({ ...place, label: menuWords.nodeMenu, items: [
        { id: 'add-node', label: menuWords.addNode, items: addItems(point) },
        { id: 'rename', label: labels.rename, keys: 'F2', run: () => startLabel(id) },
        { id: 'pass-through', label: labels.passThrough, keys: 'Ctrl+P', checked: node.pass_through === true, run: () => run(togglePass(at, composition, [id])) },
        { id: 'delete', label: menuWords.deleteNode, keys: 'Delete', disabled: node.kind === 'media_out', run: () => run(removeNodes(at, composition, [id])) },
      ] });
      return;
    }
    setMenu({ ...place, label: menuWords.graphMenu, items: graphItems(point) });
  };

  const closeMenu = useCallback(() => setMenu(null), []);
  const labelNode: PlacedFusion | null = labelling ? layout.nodes.find(each => each.node.id === labelling.id) ?? null : null;
  const litOf = (id: string) => (preview?.port?.node === id ? portName(preview.port) : null);

  return <div className="color-nodes-body fusion-nodes-body">
    <div className="color-nodes-head fusion-nodes-head">
      <h2 className="overline">{colorLabels.nodeEditor}</h2>
      <div className="fusion-add" role="group" aria-label={menuWords.addNode}>
        {ADDABLE.map((kind) => {
          const Icon = ADD_ICONS[kind];
          return <button key={kind} type="button" className="color-tool" title={`${menuWords.addNode}: ${labels.kinds[kind]}`} onClick={() => add(kind, null)}
            data-test={`add-${kind}`}><Icon size={14} aria-hidden="true" />{labels.kinds[kind]}</button>;
        })}
      </div>
      <span className="color-gap" />
      <input type="range" className="color-node-zoom" aria-label={menuWords.zoom} min={ZOOM_LEAST} max={ZOOM_MOST} step={0.05} value={zoom}
        onChange={event => setZoom(Number(event.target.value))} onPointerUp={event => event.currentTarget.blur()} data-test="node-zoom" />
      <button type="button" className="icon-btn" aria-label={menuWords.graphMenu} title={menuWords.graphMenu} aria-haspopup="menu"
        onClick={(event) => { const box = event.currentTarget.getBoundingClientRect(); setMenu({ x: box.left, y: box.bottom, label: menuWords.graphMenu, items: graphItems(null) }); }}
        data-test="fusion-graph-menu"><Ellipsis size={14} /></button>
    </div>
    {!has && <p className="caption fusion-first" role="status" data-test="fusion-first">{words.hint.studio.fusionPage.first}</p>}
    {editing.problem && <div className="color-nodes-problem" role="alert" data-test="node-problem"><span>{editing.problem}</span>
      <button type="button" className="icon-btn" aria-label={words.label.close} onClick={() => editing.setProblem(null)}><X size={13} /></button></div>}
    <div ref={canvas} className="color-nodes-canvas" data-test="node-canvas"
      onPointerDown={pointerDown} onPointerMove={pointerMove} onPointerUp={pointerUp} onPointerCancel={cancelDrag} onLostPointerCapture={cancelDrag}
      onPointerLeave={() => { if (!drag.current && hover) setHover(null); }} onContextMenu={contextMenu}>
      <div className="color-graph-area">
        <svg className="color-graph fusion-graph" width={layout.width * zoom} height={layout.height * zoom} viewBox={`0 0 ${layout.width} ${layout.height}`}
          role="group" aria-label={colorLabels.nodeEditor} data-test="fusion-graph">
          <g className="color-links">
            {layout.links.map((link) => {
              const ends = linkEnds(layout, link);
              const name = linkName(link);
              if (!ends || name === preview?.detached) return null;
              return <LinkLine key={name} name={name} input={link.input} fromX={ends.from.x} fromY={ends.from.y} toX={ends.to.x} toY={ends.to.y}
                chosen={pickedLink === name} replacing={preview?.replacing === name} hover={hover?.link === name ? hover.half : null} />;
            })}
            {preview?.wire && <path className="color-link color-wire fusion-link" d={linkPath(preview.wire.from, preview.wire.to)} data-test="wire" />}
          </g>
          {layout.nodes.map(({ node, x, y }) => <NodeCard key={node.id} node={node} x={x} y={y} name={names.get(node.id) ?? node.id} kind={labels.kinds[node.kind]}
            current={node.id === selection.current} chosen={node.id !== selection.current && selection.others.includes(node.id)} lit={litOf(node.id)}
            keyed={hasKeys(node) ? colorLabels.keyframesPanel.keyed : null} passed={node.pass_through ? labels.passedThrough : null} ports={portNames}
            onLabel={startLabel} />)}
          {preview?.marquee && <rect className="color-marquee" data-test="marquee" x={Math.min(preview.marquee.a.x, preview.marquee.b.x)}
            y={Math.min(preview.marquee.a.y, preview.marquee.b.y)} width={Math.abs(preview.marquee.a.x - preview.marquee.b.x)}
            height={Math.abs(preview.marquee.a.y - preview.marquee.b.y)} />}
        </svg>
        {labelNode && labelling && <input className="field color-label-field" aria-label={menuWords.labelField} maxLength={100} autoFocus
          style={{ left: labelNode.x * zoom, top: Math.max(0, (labelNode.y + NODE_HEIGHT / 2 - 11) * zoom), width: NODE_WIDTH * zoom }} value={labelling.value}
          onChange={event => setLabelling({ ...labelling, value: event.target.value })}
          onKeyDown={(event) => {
            if (event.key === 'Enter') { event.preventDefault(); commitLabel(); }
            else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); now.current.labelling = null; setLabelling(null); }
          }}
          onBlur={commitLabel} data-test="node-label-field" />}
      </div>
    </div>
    {menu && <CoveringMenu menu={menu} onClose={closeMenu} />}
  </div>;
}
