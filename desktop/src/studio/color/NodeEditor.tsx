import { memo, useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from 'react';
import { Hand, MousePointer2, X } from 'lucide-react';
import { keyOf } from '../../app/keys';
import { useWords } from '../../app/locale';
import { editingAllowed, inStudio } from '../keys';
import type { Grade, GradeNode, Link, LinkKind, Target } from '../engine';
import type { GradeLevel } from './clips';
import { ColorMenu, type Menu, type MenuItem } from './ColorMenu';
import {
  END_RADIUS, isComplete, layoutGrade, linkEnds, linkHalves, linkName, linkNear, linkPath, NODE_HEIGHT, NODE_WIDTH, nodeAt, nodesIn, numberText,
  PORT_INSET, portAt, shownLabel, inputPoint, inputsOf, outputPoint, hasOutput, type Layout, type Placed, type PlacedNode, type Point, type Port,
} from './graph';
import {
  addedCorrector, addInput, addLoose, addNode, composite, COMPOSITES, connect, deleteNodes, disconnect, extractNode, insertOnLink, linkIntoNewInput,
  morph, nodeKey, pasteNode, reconnect, relabel, reset, resetShotMatch, restoreOriginal, stepNode, swapNodes, toggleAll, toggleNodes, type AddHow, type NodeCommand, type Worked,
} from './nodeEdits';
import type { GradeEditing } from './useGradeEditing';
import './nodes.css';

/** How far the pointer goes before a press becomes a drag. */
const DRAG_PX = 3;
/** The least and the most that the editor zooms (p. 3311), and how much a step of Zoom In or Out is. */
export const ZOOM_LEAST = 0.25;
export const ZOOM_MOST = 3;
const ZOOM_STEP = 1.25;
const NO_PLACES: Placed = {};
const NO_IDS: ReadonlySet<string> = new Set();

/**
 * What the node editor keeps that the project does not: where nodes were dragged to, by grade (the
 * engine keeps no place for a node; Cleanup Node Graph forgets them), how far it is zoomed, and the
 * node copied with Ctrl+C. One for the page, so that they stay while the editor is closed and opened.
 */
export class NodeViews {
  private places = new Map<string, Placed>();
  zoom = 1;
  copied: { grade: Target; node: string } | null = null;

  placed(grade: string): Placed { return this.places.get(grade) ?? NO_PLACES; }
  place(grade: string, moved: Placed): void { this.places.set(grade, { ...this.placed(grade), ...moved }); }
  /** Cleanup Node Graph (p. 3321): every node of the grade where the editor lays it out. */
  cleanup(grade: string): void { this.places.delete(grade); }
}

/** The badge of a shot match on a node, a small grid of dots as in p. 3048's picture. */
export function MatchedMark({ x, y, title }: { x: number; y: number; title: string }) {
  const dots = [0, 1, 2].flatMap(row => [0, 1, 2].map(column => [x + column * 3.5, y + row * 3.5] as const));
  return <g className="color-node-matched" data-test="node-matched"><title>{title}</title>
    {dots.map(([cx, cy]) => <circle key={`${cx}-${cy}`} cx={cx} cy={cy} r={1.1} />)}</g>;
}

/** The ports of a node: inputs on its left side, outputs on its right; the picture's green, the key's blue (p. 3311). */
function Ports({ placed, lit }: { placed: PlacedNode; lit: string | null }) {
  const { node } = placed;
  const ports = [];
  for (const kind of ['rgb', 'key'] as const) {
    for (let input = 0; input < inputsOf(node, kind); input++) {
      const at = inputPoint(placed, kind, input);
      const name = `in:${kind}:${input}`;
      ports.push(<path key={name} className="color-port" data-kind={kind} data-port={name} data-lit={lit === name || undefined}
        d={`M ${at.x - 1} ${at.y - 5} L ${at.x + 6} ${at.y} L ${at.x - 1} ${at.y + 5} Z`} />);
    }
    if (hasOutput(node, kind)) {
      const at = outputPoint(placed, kind);
      const name = `out:${kind}`;
      ports.push(<rect key={name} className="color-port" data-kind={kind} data-port={name} data-lit={lit === name || undefined}
        x={at.x - 5} y={at.y - 4} width={7} height={8} />);
    }
  }
  return <>{ports}</>;
}

/** What the graph shows besides the grade: what is chosen, and what a drag is about to do. */
interface Shown {
  current: string | null;
  others: ReadonlySet<string>;
  /** The link chosen (orange), by `linkName`. */
  link: string | null;
  /** The half of a link under the pointer: the second takes its end along when dragged (blue). */
  hover: { link: string; half: 'start' | 'end' } | null;
  /** A link that a drag would put out of its place (orange), and one being taken along (not drawn). */
  replacing: string | null;
  detached: string | null;
  /** The port, and the node, that a drag would end on. */
  port: Port | null;
  node: string | null;
  /** A link being drawn from a port to the pointer. */
  wire: { from: Point; to: Point; kind: LinkKind } | null;
  marquee: { a: Point; b: Point } | null;
}

/**
 * The node editor's graph, as DaVinci Resolve draws it (Reference Manual 20, pp. 3309–3313): the
 * source on the left, each node a card with its number (01, 02 … in the order of the grade) and
 * its label above it, links of the picture and of the key drawn apart, and the output on the right.
 * The current node is orange, the others chosen white (p. 3314). Each link has two halves to take
 * hold of (p. 3319). A grade that a node is not connected in colours nothing, and says so.
 */
const GradeGraph = memo(function GradeGraph({ layout, zoom, shown, onLabel }: {
  layout: Layout; zoom: number; shown: Shown; onLabel: (node: string) => void;
}) {
  const words = useWords();
  const labels = words.label.studio.colorPage;
  return <svg className="color-graph" width={layout.width * zoom} height={layout.height * zoom} viewBox={`0 0 ${layout.width} ${layout.height}`}
    role="group" aria-label={labels.nodeEditor} data-test="grade-graph">
    <g className="color-links">
      {layout.links.map((link) => {
        const ends = linkEnds(layout, link);
        const name = linkName(link);
        if (!ends || name === shown.detached) return null;
        const [first, second] = linkHalves(ends.from, ends.to);
        const hover = shown.hover?.link === name ? shown.hover.half : undefined;
        return <g key={name}>
          <path className="color-link" data-kind={link.kind} data-link={name} d={linkPath(ends.from, ends.to)}
            data-chosen={shown.link === name || undefined} data-replacing={shown.replacing === name || undefined} data-hover={hover} />
          <path className="color-link-hit" data-link-hit={name} data-half="start" d={first} />
          <path className="color-link-hit" data-link-hit={name} data-half="end" d={second} />
        </g>;
      })}
      {shown.wire && <path className="color-link color-wire" data-kind={shown.wire.kind} d={linkPath(shown.wire.from, shown.wire.to)} data-test="wire" />}
    </g>
    <g className="color-end" data-end="source" data-test="node-source">
      <title>{labels.nodeSource}</title>
      <circle cx={layout.source.x} cy={layout.source.y} r={END_RADIUS} />
    </g>
    <g className="color-end" data-end="output" data-test="node-output" data-lit={shown.port?.node === 'output' || undefined}>
      <title>{labels.nodeOutput}</title>
      <circle cx={layout.output.x} cy={layout.output.y} r={END_RADIUS} />
    </g>
    {layout.nodes.map((placed) => {
      const { node, number, x, y } = placed;
      const name = node.kind === 'corrector' ? node.label : node.label || labels.kinds[node.kind];
      const matched = node.kind === 'corrector' && node.params.shot_match != null;
      const lit = shown.port?.node === node.id ? (shown.port.side === 'in' ? `in:${shown.port.kind}:${shown.port.input}` : `out:${shown.port.kind}`) : null;
      return <g key={node.id} className="color-node" data-node={node.id} data-number={number} data-kind={node.kind}
        data-enabled={node.enabled} data-current={node.id === shown.current || undefined}
        data-chosen={(node.id !== shown.current && shown.others.has(node.id)) || undefined} data-target={shown.node === node.id || undefined}>
        <title>{[`${numberText(number)}`, name, labels.kinds[node.kind], node.enabled ? '' : labels.disabled, matched ? labels.matched : ''].filter(Boolean).join(' · ')}</title>
        {name && <text className="color-node-label" x={x + NODE_WIDTH / 2} y={y - 6} textAnchor="middle" data-test="node-label"
          onDoubleClick={() => onLabel(node.id)}>{shownLabel(name)}</text>}
        <rect className="color-node-card" x={x} y={y} width={NODE_WIDTH} height={NODE_HEIGHT} rx={5} />
        <rect className="color-node-number-hit" x={x + 3} y={y + 2} width={24} height={PORT_INSET + 6} data-number-hit="" />
        <text className="color-node-number" x={x + 9} y={y + PORT_INSET + 4} data-test="node-number">{numberText(number)}</text>
        {node.kind !== 'corrector' && <text className="color-node-kind" x={x + NODE_WIDTH / 2} y={y + NODE_HEIGHT / 2 + 4} textAnchor="middle">{labels.kinds[node.kind]}</text>}
        {matched && <MatchedMark x={x + NODE_WIDTH - 20} y={y + NODE_HEIGHT - 13} title={labels.matched} />}
        <Ports placed={placed} lit={lit} />
      </g>;
    })}
    {shown.marquee && <rect className="color-marquee" data-test="marquee" x={Math.min(shown.marquee.a.x, shown.marquee.b.x)} y={Math.min(shown.marquee.a.y, shown.marquee.b.y)}
      width={Math.abs(shown.marquee.a.x - shown.marquee.b.x)} height={Math.abs(shown.marquee.a.y - shown.marquee.b.y)} />}
  </svg>;
});

/** A press in the graph, and what it becomes as the pointer moves. */
type Drag =
  | { kind: 'node'; id: string; ids: string[]; start: Point; moved: boolean; mode: 'move' | 'swap' | 'copy'; number: boolean; ctrl: boolean; shift: boolean; base: Layout }
  /** A link drawn from a port; `detached` is the link whose end was taken up, which goes with it. */
  | { kind: 'wire'; from: Port; at: Point; start: Point; moved: boolean; detached: Link | null; base: Layout }
  | { kind: 'half'; link: Link; half: 'start' | 'end'; start: Point; moved: boolean; base: Layout }
  | { kind: 'marquee'; start: Point; moved: boolean; additive: boolean; base: Layout }
  | { kind: 'pan'; x: number; y: number; left: number; top: number };

const parsePort = (node: string, name: string): Port => {
  const [side, kind, input] = name.split(':');
  return { node, side: side as Port['side'], kind: kind as LinkKind, input: Number(input ?? 0) };
};
const linkInto = (layout: Layout, port: Port) => layout.links.find(link => link.to === port.node && link.kind === port.kind && link.input === port.input) ?? null;
const isMixerOf = (node: GradeNode | undefined, kind: LinkKind) => node !== undefined
  && (((node.kind === 'parallel_mixer' || node.kind === 'layer_mixer') && kind === 'rgb') || (node.kind === 'key_mixer' && kind === 'key'));

/**
 * The node editor of the Color page (Reference Manual 20, Chapter 143; the interaction reference,
 * §2): which of the grades of the current clip it shows and edits (Clip or Timeline, and for a clip
 * in a group its Group Pre-Clip and Group Post-Clip, p. 3303), and that grade, edited as DaVinci
 * Resolve's is:
 *
 * - nodes added with Alt+S, Shift+S, Alt+K, Alt+P and Alt+L, or from the right-click menu, and a
 *   loose corrector from the menu of the background (p. 3318);
 * - a node chosen with a click, more with Ctrl+click (Shift+click adds too) or a box drawn round
 *   them (p. 3314); moved by dragging; deleted with Delete; extracted with E; enabled or disabled
 *   with Ctrl+D, a click on its number, or all of them with Alt+D (p. 3315); reset with Shift+Home,
 *   Ctrl+Shift+Home and Ctrl+Home (p. 3316); labelled from the menu or by a double-click (p. 3313);
 * - links drawn from an output to an input of the same kind, the end of one dragged to another
 *   input or off to take it away, its second half clicked to delete it, its first half to choose it
 *   for Delete (p. 3319); a link dropped on a mixer gives it an input more;
 * - a node's settings copied onto another with Alt+drag or Ctrl+C and Ctrl+V, swapped with
 *   Ctrl+drag (p. 3320); a loose node dropped on a link goes into it (p. 3319).
 *
 * Every change is one transaction, undone in the one history. Where a node lies, how far the editor
 * is zoomed and which nodes are chosen are the view's: the project keeps none of them.
 */
export function NodeEditor({ grade, level, levels, onLevel, current, onCurrent, hint, target, gradeKey, views, focused, editing }: {
  grade: Grade | null;
  level: GradeLevel;
  levels: readonly GradeLevel[];
  onLevel: (level: GradeLevel) => void;
  current: string | null;
  onCurrent: (node: string) => void;
  /** What is said where there is no grade to show. */
  hint: string;
  /** The grade shown, as the engine names it, which the edits change; null where there is none. */
  target: Target | null;
  /** The grade shown, as the view keeps what it knows of it (its level and its clip or timeline). */
  gradeKey: string;
  views: NodeViews;
  /** Whether the node editor has the focus of the page (p. 2967): Delete, Ctrl+C, Ctrl+V and H are its then. */
  focused: boolean;
  editing: GradeEditing;
}) {
  const words = useWords();
  const labels = words.label.studio.colorPage;
  const menuWords = labels.nodeMenu;
  const said = words.hint.studio.colorPage;
  const complete = grade === null || isComplete(grade);
  const canvas = useRef<HTMLDivElement>(null);
  const [, redraw] = useState(0);
  const [zoom, setZoomState] = useState(views.zoom);
  const [tool, setTool] = useState<'select' | 'pan'>('select');
  const [chosen, setChosen] = useState<{ key: string; ids: ReadonlySet<string> }>({ key: gradeKey, ids: NO_IDS });
  const [picked, setPicked] = useState<{ key: string; link: string } | null>(null);
  const [hover, setHover] = useState<Shown['hover']>(null);
  const [preview, setPreview] = useState<{ moved?: Placed; wire?: Shown['wire']; port?: Port | null; node?: string | null; replacing?: string | null; detached?: string | null; marquee?: Shown['marquee'] } | null>(null);
  const [menu, setMenu] = useState<Menu | null>(null);
  const [labelling, setLabelling] = useState<{ key: string; id: string; value: string } | null>(null);
  const drag = useRef<Drag | null>(null);
  // The grade as it was when it was first shown, for Original Memory (p. 3317): another clip or level shown takes it anew.
  const original = useRef<{ key: string; grade: Grade } | null>(null);
  if (grade !== null && original.current?.key !== gradeKey) original.current = { key: gradeKey, grade };

  const placed = views.placed(gradeKey);
  const base = useMemo(() => (grade === null ? null : layoutGrade(grade, placed)), [grade, placed]);
  const layout = useMemo(() => (grade === null || !preview?.moved ? base : layoutGrade(grade, { ...placed, ...preview.moved })), [grade, base, placed, preview?.moved]);
  const others = chosen.key === gradeKey ? chosen.ids : NO_IDS;
  const pickedLink = picked?.key === gradeKey && grade?.links.some(link => linkName(link) === picked.link) ? picked.link : null;
  const selected = useMemo(() => (grade === null ? [] : grade.nodes.filter(node => node.id === current || others.has(node.id)).map(node => node.id)),
    [grade, current, others]);
  const setOthers = useCallback((ids: Iterable<string>) => setChosen({ key: gradeKey, ids: new Set(ids) }), [gradeKey]);

  const setZoom = useCallback((next: number, around?: { client: Point; graph: Point }) => {
    const value = Math.min(ZOOM_MOST, Math.max(ZOOM_LEAST, next));
    views.zoom = value;
    setZoomState(value);
    const element = canvas.current;
    if (element && around) {
      const box = element.getBoundingClientRect();
      element.scrollLeft = around.graph.x * value - (around.client.x - box.left);
      element.scrollTop = around.graph.y * value - (around.client.y - box.top);
    }
  }, [views]);
  const zoomToWindow = useCallback(() => {
    const element = canvas.current;
    if (!element || !base) return;
    setZoom(Math.min(element.clientWidth / base.width, (element.clientHeight || element.clientWidth) / base.height));
  }, [base, setZoom]);

  // What the handlers of keys and of the pointer read, as it is now.
  const now = useRef({ grade, target, gradeKey, current, selected, pickedLink, focused, editing, onCurrent, zoom, menu, labelling, tool, base });
  now.current = { grade, target, gradeKey, current, selected, pickedLink, focused, editing, onCurrent, zoom, menu, labelling, tool, base };

  /** Send a command; where it adds a corrector, that one becomes current (p. 3318), and goes where it was asked for. */
  const run = useCallback((worked: Worked, at?: Point) => {
    const { editing: sending, gradeKey: key, onCurrent: choose } = now.current;
    sending.run(worked, (applied) => {
      const added = addedCorrector(applied.done);
      if (added === null) return;
      if (at) { views.place(key, { [added]: at }); redraw(count => count + 1); }
      if (now.current.gradeKey === key) choose(added);
    });
  }, [views]);

  const add = useCallback((how: AddHow) => {
    const { target: grade, current: node } = now.current;
    if (grade) run(addNode(how, grade, node));
  }, [run]);
  const startLabel = useCallback((id: string) => {
    const node = now.current.grade?.nodes.find(each => each.id === id);
    if (node) setLabelling({ key: now.current.gradeKey, id, value: node.label });
  }, []);
  const commitLabel = useCallback(() => {
    const { grade, target, labelling: writing } = now.current;
    // Enter, and then the blur of the field as it goes, commit once.
    now.current.labelling = null;
    setLabelling(null);
    if (!grade || !target || !writing || writing.key !== now.current.gradeKey) return;
    run(relabel(target, grade, writing.id, writing.value.trim()));
  }, [run]);

  /** Do what a key asks (see `nodeKey`); whether it was the editor's to do. */
  const perform = useCallback((command: NodeCommand): boolean => {
    const state = now.current;
    const { grade, target } = state;
    if (command.kind === 'cancel') {
      if (drag.current) { drag.current = null; setPreview(null); return true; }
      if (state.pickedLink) { setPicked(null); return true; }
      return false;
    }
    if (!grade || !target) return false;
    switch (command.kind) {
      case 'add': add(command.how); return true;
      case 'extract': run(extractNode(target, state.current)); return true;
      case 'enable': run(toggleNodes(target, grade, state.selected)); return true;
      case 'enableAll': run(toggleAll(target, grade)); return true;
      case 'reset': run(reset(command.how, target, grade, state.current)); return true;
      case 'original':
        if (original.current?.key === state.gradeKey) run(restoreOriginal(target, grade, original.current.grade));
        return true;
      case 'step': {
        const to = stepNode(grade, state.current, command.direction);
        if (to !== null) { state.onCurrent(to); setOthers([]); }
        return true;
      }
      default: break;
    }
    // Delete, copy, paste and H are the node editor's where it has the focus (p. 3286).
    if (!state.focused) return false;
    switch (command.kind) {
      case 'delete': {
        const link = state.pickedLink === null ? undefined : grade.links.find(each => linkName(each) === state.pickedLink);
        if (link) { setPicked(null); run(disconnect(target, link)); return true; }
        // Backspace deletes no node: the manual names Forward Delete alone (p. 3318).
        if (command.key === 'Backspace') return false;
        run(deleteNodes(target, grade, state.selected));
        return true;
      }
      case 'copy':
        if (state.current !== null) views.copied = { grade: target, node: state.current };
        return true;
      case 'paste':
        if (views.copied && state.current !== null) run(pasteNode(views.copied.grade, views.copied.node, target, state.current));
        return true;
      case 'tool': setTool(old => (old === 'select' ? 'pan' : 'select')); return true;
      default: return false;
    }
  }, [add, run, setOthers, views]);

  // The keys of the node editor. They are taken before the keys of the transport (Shift+Home would
  // go to the first frame there), while nothing is written in, no menu is open and no dialog is.
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key) || now.current.menu || now.current.labelling) return;
      const command = nodeKey(key);
      if (command && perform(command)) event.preventDefault();
    };
    window.addEventListener('keydown', down, true);
    return () => window.removeEventListener('keydown', down, true);
  }, [perform]);

  // The wheel zooms about the pointer (the manual does not say; the interaction reference, §2.1).
  useEffect(() => {
    const element = canvas.current;
    if (!element) return undefined;
    const wheel = (event: WheelEvent) => {
      if (event.deltaY === 0 || now.current.base === null) return;
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
  /** The port under an element, or near the point: a port of a node, the source's output or the output's input. */
  const portUnder = (element: Element, point: Point, layoutNow: Layout): Port | null => {
    const portElement = element.closest<SVGElement>('[data-port]');
    const nodeElement = element.closest<SVGElement>('[data-node]');
    if (portElement && nodeElement) return parsePort(nodeElement.dataset.node!, portElement.dataset.port!);
    const end = element.closest<SVGElement>('[data-end]')?.dataset.end;
    if (end === 'source') return { node: 'source', side: 'out', kind: 'rgb', input: 0 };
    if (end === 'output') return { node: 'output', side: 'in', kind: 'rgb', input: 0 };
    return portAt(layoutNow, point);
  };

  const pointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    const layoutNow = base;
    if (layoutNow === null || target === null || grade === null) return;
    const element = event.target as Element;
    if (element.closest('.color-label-field')) return;
    if (event.button === 1 || (event.button === 0 && tool === 'pan')) {
      event.preventDefault();
      drag.current = { kind: 'pan', x: event.clientX, y: event.clientY, left: canvas.current!.scrollLeft, top: canvas.current!.scrollTop };
      canvas.current!.setPointerCapture?.(event.pointerId ?? 1);
      return;
    }
    if (event.button !== 0) return;
    const point = pointOf(event);
    const ctrl = event.ctrlKey || event.metaKey;
    const port = portUnder(element, point, layoutNow);
    const hit = element.closest<SVGElement>('[data-link-hit]');
    const nodeElement = element.closest<SVGElement>('[data-node]');
    if (port) {
      // From an output, a new link; from an input that has one, the end of that link, taken up.
      const into = port.side === 'in' ? linkInto(layoutNow, port) : null;
      const from = into ? { node: into.from, side: 'out' as const, kind: into.kind, input: 0 } : port;
      drag.current = { kind: 'wire', from, at: point, start: point, moved: false, detached: into, base: layoutNow };
    } else if (hit) {
      const link = layoutNow.links.find(each => linkName(each) === hit.dataset.linkHit);
      if (link) drag.current = { kind: 'half', link, half: hit.dataset.half as 'start' | 'end', start: point, moved: false, base: layoutNow };
    } else if (nodeElement) {
      const id = nodeElement.dataset.node!;
      const mode = event.altKey ? 'copy' : ctrl ? 'swap' : 'move';
      const inSelection = selected.includes(id);
      if (!ctrl && !event.shiftKey) {
        // A press makes the node current; the others chosen stay chosen while it may yet drag them along.
        if (id !== current) {
          onCurrent(id);
          setOthers(inSelection ? selected.filter(each => each !== id) : []);
        }
      }
      const ids = mode === 'move' && inSelection && !ctrl ? selected : [id];
      drag.current = { kind: 'node', id, ids, start: point, moved: false, mode, number: element.closest('[data-number-hit], [data-test="node-number"]') !== null,
        ctrl, shift: event.shiftKey, base: layoutNow };
    } else {
      drag.current = { kind: 'marquee', start: point, moved: false, additive: ctrl || event.shiftKey, base: layoutNow };
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
      // The second half of a link takes its end along (p. 3319); the first does nothing as it is dragged.
      if (press.half === 'start') return;
      drag.current = { kind: 'wire', from: { node: press.link.from, side: 'out', kind: press.link.kind, input: 0 }, at: point, start: press.start, moved: true,
        detached: press.link, base: press.base };
    }
    const now2 = drag.current!;
    if (now2.kind === 'node') {
      const dx = point.x - now2.start.x;
      const dy = point.y - now2.start.y;
      const moved: Record<string, Point> = {};
      for (const id of now2.ids) {
        const was = now2.base.nodes.find(each => each.node.id === id);
        if (was) moved[id] = { x: was.x + dx, y: was.y + dy };
      }
      const on = now2.mode === 'move' ? null : nodeAt({ ...now2.base, nodes: now2.base.nodes.filter(each => each.node.id !== now2.id) }, point)?.node.id ?? null;
      const link = now2.mode === 'move' && now2.ids.length === 1 ? linkNear(now2.base, point) : null;
      setPreview({ moved, node: on, replacing: link ? linkName(link) : null });
    } else if (now2.kind === 'wire') {
      now2.at = point;
      const fromNode = now2.from.node;
      const port = portAt(now2.base, point, each => each.kind === now2.from.kind && each.side !== now2.from.side && each.node !== fromNode);
      const fromEnd = now2.from.side === 'out' ? portPoint(now2.base, now2.from) : point;
      const toEnd = now2.from.side === 'out' ? point : portPoint(now2.base, now2.from);
      const taken = port && port.side === 'in' ? linkInto(now2.base, port) : null;
      const mixer = !port && now2.from.side === 'out' ? nodeAt(now2.base, point) : null;
      setPreview({
        wire: { from: fromEnd ?? point, to: toEnd ?? point, kind: now2.from.kind }, port,
        node: mixer && mixer.node.id !== fromNode && isMixerOf(mixer.node, now2.from.kind) ? mixer.node.id : null,
        replacing: taken && (!now2.detached || linkName(taken) !== linkName(now2.detached)) ? linkName(taken) : null,
        detached: now2.detached ? linkName(now2.detached) : null,
      });
    } else if (now2.kind === 'marquee') {
      setPreview({ marquee: { a: now2.start, b: point } });
    }
  };

  const pointerUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    const press = drag.current;
    drag.current = null;
    setPreview(null);
    if (canvas.current?.hasPointerCapture?.(event.pointerId ?? 1)) canvas.current.releasePointerCapture(event.pointerId ?? 1);
    if (!press || press.kind === 'pan' || grade === null || target === null) return;
    const point = pointOf(event);
    if (press.kind === 'node') {
      if (!press.moved) {
        if (press.number && !press.ctrl && !press.shift) { run(toggleNodes(target, grade, [press.id])); return; }
        if (press.ctrl) {
          // Ctrl+click chooses a node more, or one chosen no more; the current one stays chosen (p. 3315).
          if (press.id !== current) setOthers(others.has(press.id) ? [...others].filter(each => each !== press.id) : [...others, press.id]);
          return;
        }
        if (press.shift) { if (press.id !== current) setOthers([...others, press.id]); return; }
        setOthers([]);
        return;
      }
      const dx = point.x - press.start.x;
      const dy = point.y - press.start.y;
      if (press.mode !== 'move') {
        const on = nodeAt({ ...press.base, nodes: press.base.nodes.filter(each => each.node.id !== press.id) }, point)?.node.id;
        if (on) {
          run(press.mode === 'copy' ? pasteNode(target, press.id, target, on) : swapNodes(target, grade, press.id, on));
          return;
        }
      }
      const moved: Record<string, Point> = {};
      for (const id of press.ids) {
        const was = press.base.nodes.find(each => each.node.id === id);
        if (was) moved[id] = { x: was.x + dx, y: was.y + dy };
      }
      views.place(gradeKey, moved);
      redraw(count => count + 1);
      if (press.mode === 'move' && press.ids.length === 1) {
        const link = linkNear(press.base, point);
        if (link) run(insertOnLink(target, grade, press.id, link));
      }
      return;
    }
    if (press.kind === 'half') {
      if (press.moved) return;
      // The second half clicked deletes the link; the first chooses it, for Delete (p. 3319).
      if (press.half === 'end') run(disconnect(target, press.link));
      else setPicked({ key: gradeKey, link: linkName(press.link) });
      return;
    }
    if (press.kind === 'marquee') {
      setPicked(null);
      if (!press.moved) { setOthers([]); return; }
      const inBox = nodesIn(press.base, press.start, point).filter(each => each !== current);
      setOthers(press.additive ? [...others, ...inBox] : inBox);
      return;
    }
    // A link drawn: into an input of its kind, from an output of its kind, or onto a mixer for an input more.
    if (!press.moved) return;
    const fromNode = press.from.node;
    const port = portAt(press.base, point, each => each.kind === press.from.kind && each.side !== press.from.side && each.node !== fromNode);
    if (press.from.side === 'out') {
      if (port) {
        run(press.detached ? reconnect(target, press.detached, { to: port.node, input: port.input })
          : connect(target, { from: fromNode, to: port.node, kind: press.from.kind, input: port.input }));
        return;
      }
      const on = nodeAt(press.base, point)?.node;
      if (on && on.id !== fromNode && isMixerOf(on, press.from.kind)) {
        const more = linkIntoNewInput(target, grade, on.id, fromNode, press.from.kind);
        if (more && Array.isArray(more)) run(press.detached ? [...disconnect(target, press.detached), ...more] : more);
        return;
      }
      // The end of a link dragged off into nothing takes the link away.
      if (press.detached) run(disconnect(target, press.detached));
      return;
    }
    if (port) run(connect(target, { from: port.node, to: fromNode, kind: press.from.kind, input: press.from.input }));
  };

  const pointerCancel = () => { drag.current = null; setPreview(null); };

  const contextMenu = (event: ReactMouseEvent<HTMLDivElement>) => {
    event.preventDefault();
    if (base === null || target === null || grade === null) return;
    const element = event.target as Element;
    const point = pointOf(event);
    const place = { x: event.clientX, y: event.clientY };
    const hit = element.closest<SVGElement>('[data-link-hit], [data-link]');
    const nodeElement = element.closest<SVGElement>('[data-node]');
    if (hit && !nodeElement) {
      const name = hit.dataset.linkHit ?? hit.dataset.link;
      const link = grade.links.find(each => linkName(each) === name);
      if (!link) return;
      setMenu({ ...place, label: menuWords.link, items: [{ id: 'delete-link', label: menuWords.deleteLink, run: () => run(disconnect(target, link)) }] });
      return;
    }
    if (nodeElement) {
      const id = nodeElement.dataset.node!;
      const node = grade.nodes.find(each => each.id === id);
      if (!node) return;
      if (id !== current) { onCurrent(id); setOthers(selected.includes(id) ? selected.filter(each => each !== id) : []); }
      setMenu({ ...place, label: menuWords.nodeMenu, items: nodeItems(node) });
      return;
    }
    setMenu({ ...place, label: menuWords.graphMenu, items: [
      { id: 'add-node', label: menuWords.addNode, items: [{ id: 'add-loose', label: labels.kinds.corrector, run: () => run(addLoose(target), point) }] },
      { id: 'reset-all', label: menuWords.resetAll, keys: 'Ctrl+Home', disabled: grade.nodes.length === 0, run: () => run(reset('all', target, grade, null)) },
      { id: 'cleanup', label: menuWords.cleanup, run: () => { views.cleanup(gradeKey); redraw(count => count + 1); } },
      { id: 'zoom-in', label: menuWords.zoomIn, run: () => setZoom(zoom * ZOOM_STEP) },
      { id: 'zoom-out', label: menuWords.zoomOut, run: () => setZoom(zoom / ZOOM_STEP) },
      { id: 'zoom-window', label: menuWords.zoomToWindow, run: zoomToWindow },
      { id: 'zoom-original', label: menuWords.originalSize, run: () => setZoom(1) },
    ] });
  };

  /** The right-click menu of a node (the interaction reference, §2.6). */
  const nodeItems = (node: GradeNode): MenuItem[] => {
    if (target === null || grade === null) return [];
    const corrector = node.kind === 'corrector';
    const items: MenuItem[] = [
      { id: 'add-node', label: menuWords.addNode, items: [
        { id: 'add-serial', label: menuWords.serial, keys: 'Alt+S', disabled: node.kind === 'key_mixer', run: () => add('serial') },
        { id: 'add-before', label: menuWords.before, keys: 'Shift+S', disabled: !corrector, run: () => add('before') },
        { id: 'add-parallel', label: menuWords.parallel, keys: 'Alt+P', disabled: !corrector, run: () => add('parallel') },
        { id: 'add-layer', label: menuWords.layer, keys: 'Alt+L', disabled: !corrector, run: () => add('layer') },
        // An outside node takes the key of the node before it, which waits for the keys of T21 and T22.
        { id: 'add-outside', label: menuWords.outside, keys: 'Alt+O', disabled: true },
      ] },
      { id: 'outside', label: menuWords.outside, keys: 'Alt+O', disabled: true },
      { id: 'label', label: menuWords.label, run: () => startLabel(node.id) },
      { id: 'delete', label: menuWords.deleteNode, keys: 'Delete', run: () => run(deleteNodes(target, grade, [node.id])) },
      { id: 'reset', label: menuWords.reset, keys: 'Shift+Home', run: () => run(reset('node', target, grade, node.id)) },
    ];
    if (node.kind === 'corrector' && node.params.shot_match != null) {
      items.push({ id: 'reset-match', label: menuWords.resetShotMatch, run: () => run(resetShotMatch(target, node.id)) });
    }
    if (node.kind !== 'corrector') items.push({ id: 'add-input', label: menuWords.addInput, run: () => run(addInput(target, grade, node.id)) });
    if (node.kind === 'parallel_mixer') items.push({ id: 'morph', label: menuWords.toLayer, run: () => run(morph(target, grade, node.id)) });
    if (node.kind === 'layer_mixer') {
      items.push({ id: 'morph', label: menuWords.toParallel, run: () => run(morph(target, grade, node.id)) });
      items.push({ id: 'composite', label: menuWords.composite, items: COMPOSITES.map(mode => ({
        id: `composite-${mode}`, label: labels.composites[mode], checked: node.params.composite === mode, run: () => run(composite(target, grade, node.id, mode)),
      })) });
    }
    return items;
  };

  const closeMenu = useCallback(() => setMenu(null), []);
  const labelNode = labelling && labelling.key === gradeKey ? layout?.nodes.find(each => each.node.id === labelling.id) ?? null : null;
  const shown: Shown = {
    current, others, link: pickedLink, hover, replacing: preview?.replacing ?? null, detached: preview?.detached ?? null,
    port: preview?.port ?? null, node: preview?.node ?? null, wire: preview?.wire ?? null, marquee: preview?.marquee ?? null,
  };

  return <div className="color-nodes-body">
    <div className="color-nodes-head">
      <h2 className="overline">{labels.nodeEditor}</h2>
      <div className="color-node-tools" role="group" aria-label={menuWords.tools}>
        <button type="button" className="icon-btn" aria-pressed={tool === 'select'} aria-label={menuWords.selectTool} title={`${menuWords.selectTool} (H)`}
          onClick={() => setTool('select')} data-test="node-select-tool"><MousePointer2 size={13} /></button>
        <button type="button" className="icon-btn" aria-pressed={tool === 'pan'} aria-label={menuWords.panTool} title={`${menuWords.panTool} (H)`}
          onClick={() => setTool('pan')} data-test="node-pan-tool"><Hand size={13} /></button>
        <input type="range" className="color-node-zoom" aria-label={menuWords.zoom} min={ZOOM_LEAST} max={ZOOM_MOST} step={0.05} value={zoom}
          onChange={event => setZoom(Number(event.target.value))} onPointerUp={event => event.currentTarget.blur()} data-test="node-zoom" />
      </div>
      <span className="color-gap" />
      <div className="color-level-dots" role="group" aria-label={labels.level}>
        {levels.map(each => <button key={each} type="button" className="color-level-dot" aria-pressed={each === level} aria-label={labels.levels[each]}
          title={labels.levels[each]} onClick={() => onLevel(each)} data-level={each} />)}
      </div>
      <select className="field color-level" aria-label={labels.level} value={level}
        onChange={(event) => { onLevel(event.target.value as GradeLevel); event.currentTarget.blur(); }} data-test="node-level">
        {levels.map(each => <option key={each} value={each}>{labels.levels[each]}</option>)}
      </select>
    </div>
    {!complete && <p className="color-incomplete" role="status" data-test="grade-incomplete">{said.incomplete}</p>}
    {editing.problem && <div className="color-nodes-problem" role="alert" data-test="node-problem"><span>{editing.problem}</span>
      <button type="button" className="icon-btn" aria-label={words.label.close} onClick={() => editing.setProblem(null)}><X size={13} /></button></div>}
    <div ref={canvas} className="color-nodes-canvas" data-test="node-canvas" data-tool={tool}
      onPointerDown={pointerDown} onPointerMove={pointerMove} onPointerUp={pointerUp} onPointerCancel={pointerCancel} onLostPointerCapture={pointerCancel}
      onPointerLeave={() => { if (!drag.current && hover) setHover(null); }} onContextMenu={contextMenu}>
      {grade === null || layout === null ? <p className="caption color-empty">{hint}</p> : <div className="color-graph-area">
        <GradeGraph layout={layout} zoom={zoom} shown={shown} onLabel={startLabel} />
        {grade.nodes.length === 0 && <p className="caption color-empty" data-test="no-nodes">{said.noNodes}</p>}
        {labelNode && labelling && <input className="field color-label-field" aria-label={menuWords.labelField} maxLength={100} autoFocus
          style={{ left: labelNode.x * zoom, top: Math.max(0, (labelNode.y - 24) * zoom), width: NODE_WIDTH * zoom }} value={labelling.value}
          onChange={event => setLabelling({ ...labelling, value: event.target.value })}
          onKeyDown={(event) => {
            if (event.key === 'Enter') { event.preventDefault(); commitLabel(); }
            else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); now.current.labelling = null; setLabelling(null); }
          }}
          onBlur={commitLabel} data-test="node-label-field" />}
      </div>}
    </div>
    {menu && <ColorMenu menu={menu} onClose={closeMenu} />}
  </div>;
}

/** Where a port is drawn, in a layout. */
function portPoint(layout: Layout, port: Port): Point | null {
  if (port.node === 'source') return { x: layout.source.x + END_RADIUS, y: layout.source.y };
  if (port.node === 'output') return { x: layout.output.x - END_RADIUS, y: layout.output.y };
  const placed = layout.nodes.find(each => each.node.id === port.node);
  if (!placed) return null;
  return port.side === 'in' ? inputPoint(placed, port.kind, port.input) : outputPoint(placed, port.kind);
}

