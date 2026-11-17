import { memo, useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from 'react';
import { Ellipsis, X } from 'lucide-react';
import { keyOf } from '../../app/keys';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { EngineError, type Clip, type Composition, type FusionInput, type FusionKind, type FusionLink, type FusionNode, type Op } from '../engine';
import { editingAllowed, inStudio } from '../keys';
import type { Menu, MenuItem } from '../color/ColorMenu';
import { CoveringMenu } from '../color/CoveringMenu';
import { NODE_HEIGHT, NODE_WIDTH, shownLabel, type Placed, type Point } from '../color/graph';
import { KeyedMark, ZOOM_LEAST, ZOOM_MOST } from '../color/NodeEditor';
import type { GradeEditing } from '../color/useGradeEditing';
import { refusalText } from '../inspector/refusal';
import { useAudioGesture } from '../timeline/useAudioGesture';
import { copiedNodes, keepCopied } from './copied';
import {
  ADDABLE, ADDABLE_EFFECTS, ADDABLE_MASKS, addedNode, addedNodes, addNode, connect, copyNodes, cutNodes, disconnect, fusionKey, insertOnLink, isFirst, moveNodes,
  pasteNodes, reconnect, relabel, removeComposition, removeNodes, resetComposition, restoreNodes, roomFor, togglePass, viewedAfter, withFirst, type CopiedNodes,
  type FusionAt, type FusionCommand,
} from './edits';
import {
  fusionLinkHalves, fusionLinkNear, fusionLinkPath, hasMaskInput, hasOutput, isPictureInput, layoutComposition, linkEnds, linkName, MASK_INSET, MATTE_INSETS,
  matteInputs, namesOf, nodesIn, pictureInputs, placesOf, portAt, portName, portPoint, type FusionLayout, type FusionPort, type PlacedFusion,
} from './graph';
import { ADD_ICONS, SelectTool, TOOLS, type AddableKind } from './SelectTool';
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

/** The nodes that are chosen: the one chosen last, whose settings the Inspector shows where it is the only one, and the others. */
export interface FusionSelection { current: string | null; others: readonly string[] }
export const NO_SELECTION: FusionSelection = { current: null, others: [] };
/** An editor that is given no viewer to show a node in shows none. */
const NO_VIEW = () => {};

/** Why the engine refused, where it is a refusal that changes nothing: a node dropped where it was, which is no failure. */
const isEmpty = (error: unknown): boolean => error instanceof EngineError && typeof error.data === 'object' && error.data !== null
  && (error.data as { reason?: unknown }).reason === 'empty';

/**
 * A node of a composition: a card with a stripe in the colour of its kind and its name, the inputs
 * of its pictures down its left side (a Merge's background orange above its foreground green, as
 * in Fusion, p. 2162), its mask input blue on its lower edge, where it has one (the effect mask of
 * a Merge, a Transform, a Background, a Blur, a Mosaic, a Delta Keyer and a Tracker, and the mask that comes into a mask), the
 * two matte inputs of a Delta Keyer on its upper edge, each in a colour of its own (the garbage matte a quarter of
 * the way along, the solid matte three quarters), and its output at
 * the middle of its right side. A node that is passed through is drawn dimmed, and one whose
 * settings have keys has a badge. The node that the viewer shows has a dot in the lower right corner
 * of its card, where no port and no badge lies. Drawn again only when what it shows changes: while
 * one node is dragged the others are left as they are.
 */
const NodeCard = memo(function NodeCard({ node, x, y, name, kind, current, chosen, lit, keyed, passed, viewed, ports, onLabel }: {
  node: FusionNode; x: number; y: number; name: string; kind: string; current: boolean; chosen: boolean;
  /** What the mark of the node that the viewer shows says, where this is that node. */
  viewed: string | null;
  /** What each port says of itself where the pointer rests on it: the names of the inputs, and of the output. */
  ports: Record<FusionInput | 'output', string>;
  /** The port that a link being drawn would end on, as `portName` names it, or null. */
  lit: string | null;
  /** What the badge of keys says, where the node has keys; and what a node passed through says of itself. */
  keyed: string | null; passed: string | null;
  onLabel: (node: string) => void;
}) {
  const inputs = pictureInputs(node.kind);
  const maskPort = portName({ side: 'in', input: 'mask' });
  const maskX = x + MASK_INSET;
  const foot = y + NODE_HEIGHT;
  return <g className="color-node fusion-node" data-node={node.id} data-kind={node.kind} data-current={current || undefined} data-chosen={chosen || undefined}
    data-pass-through={node.pass_through || undefined} data-viewed={viewed !== null || undefined}>
    <title>{[name, kind, passed ?? '', keyed ?? '', viewed ?? ''].filter(Boolean).join(' · ')}</title>
    <rect className="color-node-card" x={x} y={y} width={NODE_WIDTH} height={NODE_HEIGHT} rx={5} />
    <rect className="fusion-node-stripe" x={x + 1} y={y + 1} width={NODE_WIDTH - 2} height={6} rx={3} />
    <text className="fusion-node-name" x={x + NODE_WIDTH / 2} y={y + NODE_HEIGHT / 2 + 5} textAnchor="middle" data-test="node-label"
      onDoubleClick={() => onLabel(node.id)}>{shownLabel(name)}</text>
    {keyed !== null && <KeyedMark x={x + NODE_WIDTH / 2} y={y + NODE_HEIGHT - 8} title={keyed} />}
    {viewed !== null && <circle className="fusion-node-viewed" cx={x + NODE_WIDTH - 9} cy={y + NODE_HEIGHT - 9} r={3.5} data-test="node-viewed"><title>{viewed}</title></circle>}
    {inputs.map((input, at) => {
      const py = y + (NODE_HEIGHT * (at + 1)) / (inputs.length + 1);
      const port = portName({ side: 'in', input });
      return <path key={input} className="color-port fusion-port" data-input={input} data-port={port} data-lit={lit === port || undefined}
        d={`M ${x - 1} ${py - 5} L ${x + 6} ${py} L ${x - 1} ${py + 5} Z`}><title>{ports[input]}</title></path>;
    })}
    {/* The matte inputs of a Delta Keyer point down into the card from its upper edge, each at the place that `inputPoint` gives it. */}
    {matteInputs(node.kind).map((input) => {
      const px = x + MATTE_INSETS[input];
      const port = portName({ side: 'in', input });
      return <path key={input} className="color-port fusion-port" data-input={input} data-port={port} data-lit={lit === port || undefined}
        d={`M ${px - 5} ${y - 1} L ${px} ${y + 6} L ${px + 5} ${y - 1} Z`}><title>{ports[input]}</title></path>;
    })}
    {/* The mask input points up into the card from its lower edge, at the place that `inputPoint` gives it. */}
    {hasMaskInput(node.kind) && <path className="color-port fusion-port" data-input="mask" data-port={maskPort} data-lit={lit === maskPort || undefined}
      d={`M ${maskX - 5} ${foot + 1} L ${maskX} ${foot - 6} L ${maskX + 5} ${foot + 1} Z`}><title>{ports.mask}</title></path>}
    {hasOutput(node.kind) && <rect className="color-port fusion-port" data-port="out" data-lit={lit === 'out' || undefined}
      x={x + NODE_WIDTH - 5} y={y + NODE_HEIGHT / 2 - 4} width={7} height={8}><title>{ports.output}</title></rect>}
  </g>;
});

/**
 * A link of a composition, from an output to an input, with two halves to take hold of: either
 * chooses it when it is clicked, and the second takes its end along when it is dragged. A link into
 * a `mask` comes into it from below and is drawn in the colour of masks, and one into a matte of a
 * Delta Keyer from above, in the colour of that matte. Drawn again only when one of its ends moves
 * or what is said of it changes.
 */
const LinkLine = memo(function LinkLine({ name, input, fromX, fromY, toX, toY, chosen, replacing, hover }: {
  name: string; input: FusionInput; fromX: number; fromY: number; toX: number; toY: number; chosen: boolean; replacing: boolean; hover: 'start' | 'end' | null;
}) {
  const from = { x: fromX, y: fromY };
  const to = { x: toX, y: toY };
  const [first, second] = fusionLinkHalves(from, to, input);
  return <g>
    <path className="color-link fusion-link" data-link={name} data-input={input} d={fusionLinkPath(from, to, input)} data-chosen={chosen || undefined}
      data-replacing={replacing || undefined} data-hover={hover ?? undefined} />
    <path className="color-link-hit" data-link-hit={name} data-half="start" d={first} />
    <path className="color-link-hit" data-link-hit={name} data-half="end" d={second} />
  </g>;
});

/** A press in the graph, and what it becomes as the pointer moves. */
type Drag =
  /** Nodes dragged: `live` once the gesture that sends their places has begun; `still` where they may be chosen and not moved (a locked track). */
  | { kind: 'node'; id: string; ids: string[]; start: Point; moved: boolean; live: boolean; ctrl: boolean; shift: boolean; base: FusionLayout; last: Placed; still: boolean }
  /** A link drawn from a port; `detached` is the link whose end was taken up, which goes with it. */
  | { kind: 'wire'; from: FusionPort; start: Point; moved: boolean; detached: FusionLink | null; base: FusionLayout }
  | { kind: 'half'; link: FusionLink; half: 'start' | 'end'; start: Point; moved: boolean; base: FusionLayout }
  | { kind: 'marquee'; start: Point; moved: boolean; additive: boolean; base: FusionLayout }
  | { kind: 'pan'; x: number; y: number; left: number; top: number };

/** What a drag is about to do, drawn until it ends. */
interface Preview {
  moved?: Placed;
  /** The link that is drawn, and the input that it would go into, where it is over one or was begun at one: into a `mask` it is drawn as a link of a mask. */
  wire?: { from: Point; to: Point; input: FusionInput | null } | null;
  port?: FusionPort | null;
  /** The link that a link drawn would take the place of, or that the one node dragged would go into, where it is over one. */
  replacing?: string | null;
  detached?: string | null;
  marquee?: { a: Point; b: Point } | null;
}

const linkInto = (layout: FusionLayout, port: FusionPort) => layout.links.find(link => link.to === port.node && link.input === port.input) ?? null;

/**
 * The link that the one node dragged would go into where it is let go at `point`: the link of a picture nearest the pointer,
 * as in the Color page's editor, where the node is one that goes into a link (`insertOnLink`); null otherwise. A link into a
 * `mask` or into a matte of a Delta Keyer is no link of a picture, and is passed over.
 */
function linkUnder(at: FusionAt, composition: Composition, layout: FusionLayout, ids: readonly string[], point: Point): FusionLink | null {
  if (ids.length !== 1) return null;
  const link = fusionLinkNear(layout, point, each => isPictureInput(each.input) && each.from !== ids[0] && each.to !== ids[0]);
  return link !== null && insertOnLink(at, composition, ids[0], link).length > 0 ? link : null;
}

/**
 * The node editor of the Fusion page (DaVinci Resolve Reference Manual 20, Ch. 67, pp. 1316–1365):
 * the composition of the current clip, drawn and edited with what the Color page's node editor is
 * made of (its measures, the curve and the halves of a link, its style sheet, its menus and the way
 * it sends a change and says a refusal), about the nodes and the named inputs of a composition:
 *
 * - a node added from the tool bar or from the right-click menu (Merge, Transform, Background,
 *   MediaIn), where the menu was opened; with one node chosen, a node that has an input goes in
 *   after it (`addNode`), and the new node is the one chosen; an effect on a picture (Blur, Mosaic,
 *   Delta Keyer, Tracker, a group of their own in the tool bar) goes in after the node chosen in the same way; a mask
 *   (Rectangle, Ellipse, Polygon, a group of their own too) is linked into the `mask` of the node
 *   chosen instead, where that node has one and nothing is linked into it;
 * - a node chosen with a click, more with Ctrl+click or Shift+click or a box drawn round them, all
 *   with Ctrl+A; moved by dragging, which is one gesture: each move sends the places whole, the
 *   release ends it, and the whole is one step to undo (Escape takes it back);
 * - a link drawn from an output to an input, or from an input to an output; one into an input that
 *   has a link takes its place; the end of a link dragged to another input, or off to take the link
 *   away; a link chosen with a click and deleted with Delete or Backspace; the `mask` input of a
 *   node, and the garbage matte and the solid matte of a Delta Keyer, are inputs as the others are,
 *   and any output goes into them;
 * - Delete deletes the nodes chosen, joining what fed each to what it fed where that is a single
 *   chain (`removeNodes`); the MediaOut stays; a Tracker that the centre of another node follows is
 *   deleted with the centre letting go of it first, in the same transaction, and so is one cut;
 *   Ctrl+P passes the nodes chosen through; F2, a double click on its name, or its menu renames a node;
 * - Reset Fusion Composition (p. 1272) and Remove Composition, from the menu of the background and
 *   of the button at the right of the tool bar;
 * - Ctrl+C, Ctrl+X, Ctrl+V and Ctrl+D (Cmd on a Mac), and the same in the menus, copy, cut, paste
 *   and duplicate the nodes chosen, with the links among them (`copyNodes`, `pasteNodes`): what
 *   was copied is kept for the window (`copied.ts`), so it is pasted into the composition of
 *   another clip too; pasted, the nodes lie down and right of where they were, or where the menu
 *   was opened, linked to nothing else, and are the ones chosen; a paste that would give the
 *   composition more nodes than it may have is not sent, and the editor says why;
 * - one node that has no links, dragged and let go over a link of a picture, goes into that link
 *   (`insertOnLink`), in the gesture of the move; the link is drawn as one about to be replaced
 *   while the node is over it;
 * - Select Tool (Shift+Space, and in the menus): a dialog that lists the kinds that the editor
 *   adds, narrowed by what is typed, and adds the one taken as its button of the tool bar does;
 * - the key `1`, and View in the menu of a node, show the node chosen in the viewer, alone, as
 *   Fusion's left viewer does (`viewed`, `onView`): it has a dot on its card then. The same again,
 *   the key `0`, or View Output in the menus goes back to what comes out of the composition. The
 *   viewer is the page's, and looking at a node changes nothing of the project: it is done on a
 *   locked track too.
 *
 * Where the clip lies on a locked track (`locked`), nothing of it is changed, as on the Edit page:
 * the buttons and the commands that change something are off, a drag moves no node and draws no
 * link, the keys that change something do nothing, and the editor says why. Nodes are still
 * chosen, copied and looked at, and the graph is zoomed and scrolled.
 *
 * A clip without a composition is shown with the first one (`shownComposition`), and says so: the
 * first change sends it before itself (`withFirst`). Where a node lies is the project's (`pos`).
 * How far the editor is zoomed and what is chosen are the view's. What the engine refuses is said
 * above the graph, as the Color page says it; a change that changes nothing (`empty`, a node
 * dragged back to where it was) is no refusal and is not said.
 */
export function FusionNodeEditor({ bridge, session, revision, clip, composition, at, selection, onSelection, focused, editing, locked = false, viewed = null, onView = NO_VIEW }: {
  bridge: Pick<EngineBridge, 'call'>; session: number; revision: number;
  /** The node that the viewer shows, by its id; null where it shows what comes out of the composition. */
  viewed?: string | null;
  /** Have the viewer show a node, or what comes out of the composition (null). */
  onView?: (node: string | null) => void;
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
  /** Whether the clip lies on a locked track: its composition is shown, and nothing of it is changed. */
  locked?: boolean;
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
  // Select Tool, while it is open: where the node taken is put (where the menu was opened, or where the editor lays it out), and the node that it goes in after, where a menu of a node opened it.
  const [tool, setTool] = useState<{ pos: Point | null; after?: string } | null>(null);
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
  const now = useRef({ composition, clip, has, at, selection, selected, pickedLink, focused, editing, onSelection, zoom, menu, labelling, tool, base, words, locked, viewed, onView });
  now.current = { composition, clip, has, at, selection, selected, pickedLink, focused, editing, onSelection, zoom, menu, labelling, tool, base, words, locked, viewed, onView };

  // The places of nodes dragged are sent as one gesture, so that a drag is one step to undo.
  const gesture = useAudioGesture({
    bridge, session, revision, identity: `${at.timeline}/${at.clip}/fusion-nodes`, locked,
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

  // A track locked while Select Tool is open closes it: it could add nothing.
  useEffect(() => { if (locked) setTool(null); }, [locked]);

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
    const { editing: sending, at: where, has: there, locked: off } = now.current;
    // Nothing of a clip on a locked track is changed, whatever asks for it.
    if (off) return;
    const sent = withFirst(where, there, ops);
    if (sent.length > 0) sending.run(sent, applied => then?.(applied.done));
  }, []);

  /**
   * Add a node of a kind, after the one node chosen where it can go there (or after `into`, the node whose menu asks for it), at
   * `pos` or where the editor lays it out: it is the one chosen then.
   */
  const add = useCallback((kind: FusionKind, pos: Point | null, into?: string) => {
    const { composition: of, at: where, selection: chosen } = now.current;
    const after = into ?? (chosen.current !== null && chosen.others.length === 0 ? chosen.current : null);
    run(addNode(where, of, kind, after, pos), (done) => {
      const made = addedNode(done);
      if (made !== null) now.current.onSelection({ current: made, others: [] });
    });
  }, [run]);

  /** Keep nodes for Paste (Ctrl+C); whether there were any to keep. Nothing of the project is changed, so a locked track does not hinder it. */
  const copy = useCallback((ids: readonly string[]): boolean => {
    const copied = copyNodes(now.current.composition, ids);
    if (copied === null) return false;
    keepCopied(copied);
    return true;
  }, []);

  /**
   * Add nodes that were copied, at a point of the graph or down and right of where they were: they are the ones chosen then, the
   * last of them the one chosen last. A composition without room for them is left as it is, and the editor says why. Whether
   * there was anything to paste.
   */
  const paste = useCallback((copied: CopiedNodes | null, pos: Point | null): boolean => {
    const { composition: of, at: where, locked: off, editing: sending, words: said } = now.current;
    if (off || copied === null) return false;
    if (!roomFor(of, copied.nodes.length)) {
      sending.setProblem(said.hint.studio.fusionPage.tooManyNodes);
      return true;
    }
    run(pasteNodes(where, of, copied, pos), (done) => {
      const made = addedNodes(done);
      if (made.length > 0) now.current.onSelection({ current: made[made.length - 1], others: made.slice(0, -1) });
    });
    return true;
  }, [run]);

  /** Cut nodes (Ctrl+X): they are kept for Paste and taken away as Delete takes them, in one transaction. Whether there were any to cut. */
  const cut = useCallback((ids: readonly string[]): boolean => {
    const { composition: of, at: where, locked: off } = now.current;
    if (off) return false;
    const { copied, ops } = cutNodes(where, of, ids);
    if (copied === null) return false;
    keepCopied(copied);
    run(ops);
    return true;
  }, [run]);

  /** Duplicate nodes (Ctrl+D): a copy of them is pasted, and what was copied before stays kept. */
  const duplicate = useCallback((ids: readonly string[]): boolean => paste(copyNodes(now.current.composition, ids), null), [paste]);

  const startLabel = useCallback((id: string) => {
    if (now.current.locked) return;
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
    // Choosing and copying change nothing of the project: they are done on a locked track too.
    if (command.kind === 'selectAll') {
      const ids = state.composition.nodes.map(node => node.id);
      state.onSelection({ current: state.selection.current ?? ids[0] ?? null, others: ids.filter(id => id !== (state.selection.current ?? ids[0])) });
      return true;
    }
    if (command.kind === 'copy') return copy(state.selected);
    // Neither does looking at a node in the viewer: the node chosen last, or what comes out of the composition again.
    if (command.kind === 'view') {
      if (state.selection.current === null) return false;
      state.onView(viewedAfter(state.composition, state.viewed, state.selection.current));
      return true;
    }
    if (command.kind === 'viewOutput') {
      if (state.viewed === null) return false;
      state.onView(null);
      return true;
    }
    if (state.locked) return false;
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
      case 'cut': return cut(state.selected);
      case 'paste': return paste(copiedNodes(), null);
      case 'duplicate': return duplicate(state.selected);
      case 'selectTool':
        setTool({ pos: null });
        return true;
      default: return false;
    }
  }, [cancelDrag, copy, cut, duplicate, paste, run, startLabel]);

  // The keys of the node editor, while nothing is written in, no menu is open and no dialog is. The Spline Editor keeps its own keys
  // to itself: it takes them on their way up, before they come here. Shift+Space alone is taken on the way down, before the keys of
  // the transport, which would play on it whichever of the two was asked first; and not from within the Spline Editor.
  useEffect(() => {
    const take = (event: KeyboardEvent, early: boolean) => {
      if (event.defaultPrevented || !inStudio(event.target)) return;
      const key = keyOf(event);
      if (!editingAllowed(event.target, key) || now.current.menu || now.current.labelling || now.current.tool) return;
      const command = fusionKey(key);
      if (!command || (command.kind === 'selectTool') !== early) return;
      if (early && event.target instanceof Element && event.target.closest('.curve-editor')) return;
      if (perform(command)) event.preventDefault();
    };
    const down = (event: KeyboardEvent) => take(event, false);
    const first = (event: KeyboardEvent) => take(event, true);
    window.addEventListener('keydown', down);
    window.addEventListener('keydown', first, true);
    return () => {
      window.removeEventListener('keydown', down);
      window.removeEventListener('keydown', first, true);
    };
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
    // On a locked track no link is drawn or taken up: a press on a port is a press on its node.
    const port = locked ? null : portUnder(element, point, base);
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
      drag.current = { kind: 'node', id, ids, start: point, moved: false, live: false, ctrl, shift: event.shiftKey, base, last: NO_PLACES, still: locked };
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
    // On a locked track a node is chosen and not moved: its press stays a press, however far the pointer goes.
    if (press.kind === 'node' && press.still) return;
    const point = pointOf(event);
    const far = Math.hypot(point.x - press.start.x, point.y - press.start.y) * zoom >= DRAG_PX;
    if (!press.moved && !far) return;
    press.moved = true;
    if (press.kind === 'half') {
      // The second half of a link takes its end along; the first does nothing as it is dragged, and neither does on a locked track.
      if (press.half === 'start' || locked) return;
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
      const { composition: of, at: where, has: there } = now.current;
      // One node that has no links, over a link of a picture, would go into it: the link is drawn as one about to be replaced.
      const over = linkUnder(where, of, going.base, going.ids, point);
      setPreview({ moved, replacing: over ? linkName(over) : null });
      // The gesture begins with the first move that it can be sent with; what it puts back is where the nodes were, or no composition where the clip had none.
      if (!going.live) going.live = gesture.start(there ? restoreNodes(where, of, going.ids) : [{ op: 'set_fusion', ...where, value: null }]);
      if (going.live) gesture.preview(withFirst(where, there, moveNodes(where, of, moved, true)));
    } else if (going.kind === 'wire') {
      const port = endAt(going.base, going.from, point);
      const anchor = portPoint(going.base, going.from) ?? point;
      const taken = port && port.side === 'in' ? linkInto(going.base, port) : null;
      setPreview({
        wire: going.from.side === 'out' ? { from: anchor, to: point, input: port?.input ?? null } : { from: point, to: anchor, input: going.from.input }, port,
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
      // One node that has no links, let go over a link of a picture, goes into it: its two links are sent with its place.
      const into = linkUnder(at, composition, press.base, press.ids, point);
      const joined = into === null ? [] : insertOnLink(at, composition, press.id, into);
      // The release ends the gesture; where none could begin, the places are sent as one transaction, which is one step too.
      if (press.live) {
        // The links go in the last transaction of the gesture, after the place of the node, so that the whole is one step to undo.
        if (joined.length > 0) gesture.preview(withFirst(at, has, [...moveNodes(at, composition, press.last, true), ...joined]));
        gesture.end();
      } else run([...moveNodes(at, composition, press.last), ...joined]);
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

  /** The items that add a node of each kind, the effects and then the masks after the others, at a point of the graph or where the editor lays it out. */
  const addItems = (pos: Point | null, into?: string): MenuItem[] => TOOLS
    .map(kind => ({ id: `add-${kind}`, label: labels.kinds[kind], run: () => add(kind, pos, into) }));
  /** The buttons of the tool bar that add a node of each of these kinds, where the editor lays it out. */
  const addButtons = (kinds: readonly AddableKind[]) => kinds.map((kind) => {
    const Icon = ADD_ICONS[kind];
    return <button key={kind} type="button" className="color-tool" title={`${menuWords.addNode}: ${labels.kinds[kind]}`} disabled={locked} onClick={() => add(kind, null)}
      data-test={`add-${kind}`}><Icon size={14} aria-hidden="true" />{labels.kinds[kind]}</button>;
  });
  /**
   * The menu of the background: Add Node ▸, Select Tool, Paste (where the menu was opened), the two commands of the whole
   * composition, and the zoom. On a locked track only the zoom is left.
   */
  const graphItems = (pos: Point | null): MenuItem[] => [
    { id: 'add-node', label: menuWords.addNode, disabled: locked, items: addItems(pos) },
    { id: 'select-tool', label: labels.selectToolMenu, keys: 'Shift+Space', disabled: locked, run: () => setTool({ pos }) },
    { id: 'paste', label: labels.paste, keys: 'Ctrl+V', disabled: locked || copiedNodes() === null, run: () => { paste(copiedNodes(), pos); } },
    { id: 'reset-composition', label: labels.resetComposition, disabled: locked || !has || isFirst(composition), run: () => run(resetComposition(at, clip)) },
    { id: 'remove-composition', label: labels.removeComposition, disabled: locked || !has, run: () => run(removeComposition(at, clip)) },
    { id: 'view-output', label: labels.viewOutput, keys: '0', disabled: viewed === null, run: () => onView(null) },
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
      if (link) {
        setMenu({ ...place, label: menuWords.link, items: [
          { id: 'delete-link', label: menuWords.deleteLink, keys: 'Delete', disabled: locked, run: () => run(disconnect(at, link)) },
        ] });
      }
      return;
    }
    if (nodeElement) {
      const id = nodeElement.dataset.node!;
      const node = composition.nodes.find(each => each.id === id);
      if (!node) return;
      // The node right-clicked is the one chosen last: a node added from its menu goes in after it. Where it is one of several
      // that are chosen, they stay chosen, and the commands of the menu are about them all, as the keys are; else it alone is chosen.
      const acted = selected.length > 1 && selected.includes(id) ? selected : [id];
      const chosenNow: FusionSelection = { current: id, others: acted.filter(each => each !== id) };
      if (id !== selection.current || chosenNow.others.length !== selection.others.length) onSelection(chosenNow);
      now.current.selection = chosenNow;
      const copyable = copyNodes(composition, acted) !== null;
      setMenu({ ...place, label: menuWords.nodeMenu, items: [
        { id: 'add-node', label: menuWords.addNode, disabled: locked, items: addItems(point, id) },
        { id: 'select-tool', label: labels.selectToolMenu, keys: 'Shift+Space', disabled: locked, run: () => setTool({ pos: point, after: id }) },
        { id: 'cut', label: labels.cut, keys: 'Ctrl+X', disabled: locked || !copyable, run: () => { cut(acted); } },
        { id: 'copy', label: labels.copy, keys: 'Ctrl+C', disabled: !copyable, run: () => { copy(acted); } },
        { id: 'paste', label: labels.paste, keys: 'Ctrl+V', disabled: locked || copiedNodes() === null, run: () => { paste(copiedNodes(), point); } },
        { id: 'duplicate', label: labels.duplicate, keys: 'Ctrl+D', disabled: locked || !copyable, run: () => { duplicate(acted); } },
        { id: 'rename', label: labels.rename, keys: 'F2', disabled: locked, run: () => startLabel(id) },
        { id: 'pass-through', label: labels.passThrough, keys: 'Ctrl+P', checked: node.pass_through === true, disabled: locked,
          run: () => run(togglePass(at, composition, acted)) },
        // The node right-clicked in the viewer, or what comes out of the composition again: the viewer is the page's, so a locked track does not hinder it.
        { id: 'view', label: labels.view, keys: '1', checked: viewed === id, run: () => onView(viewedAfter(composition, viewed, id)) },
        { id: 'view-output', label: labels.viewOutput, keys: '0', disabled: viewed === null, run: () => onView(null) },
        { id: 'delete', label: menuWords.deleteNode, keys: 'Delete', disabled: locked || removeNodes(at, composition, acted).length === 0,
          run: () => run(removeNodes(at, composition, acted)) },
      ] });
      return;
    }
    setMenu({ ...place, label: menuWords.graphMenu, items: graphItems(point) });
  };

  const closeMenu = useCallback(() => setMenu(null), []);
  const closeTool = useCallback(() => setTool(null), []);
  const labelNode: PlacedFusion | null = labelling ? layout.nodes.find(each => each.node.id === labelling.id) ?? null : null;
  const litOf = (id: string) => (preview?.port?.node === id ? portName(preview.port) : null);

  return <div className="color-nodes-body fusion-nodes-body">
    <div className="color-nodes-head fusion-nodes-head">
      <h2 className="overline">{colorLabels.nodeEditor}</h2>
      <div className="fusion-add" role="group" aria-label={menuWords.addNode}>{addButtons(ADDABLE)}</div>
      {/* The effects on a picture, in a group of their own: one added with a node chosen goes in after it, as a Transform does. */}
      <div className="fusion-add fusion-add-effects" role="group" aria-label={labels.effects}>{addButtons(ADDABLE_EFFECTS)}</div>
      {/* The masks, in a group of their own: one added with a node chosen that has a free mask input is linked into it. */}
      <div className="fusion-add fusion-add-masks" role="group" aria-label={labels.masks}>{addButtons(ADDABLE_MASKS)}</div>
      <span className="color-gap" />
      <input type="range" className="color-node-zoom" aria-label={menuWords.zoom} min={ZOOM_LEAST} max={ZOOM_MOST} step={0.05} value={zoom}
        onChange={event => setZoom(Number(event.target.value))} onPointerUp={event => event.currentTarget.blur()} data-test="node-zoom" />
      <button type="button" className="icon-btn" aria-label={menuWords.graphMenu} title={menuWords.graphMenu} aria-haspopup="menu"
        onClick={(event) => { const box = event.currentTarget.getBoundingClientRect(); setMenu({ x: box.left, y: box.bottom, label: menuWords.graphMenu, items: graphItems(null) }); }}
        data-test="fusion-graph-menu"><Ellipsis size={14} /></button>
    </div>
    {/* Why nothing can be changed, in the words of the Inspector of the Edit page. */}
    {locked && <p className="caption fusion-locked" role="status" data-test="fusion-locked">{words.hint.studio.inspector.locked}</p>}
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
            {preview?.wire && <path className="color-link color-wire fusion-link" data-input={preview.wire.input ?? undefined}
              d={fusionLinkPath(preview.wire.from, preview.wire.to, preview.wire.input)} data-test="wire" />}
          </g>
          {layout.nodes.map(({ node, x, y }) => <NodeCard key={node.id} node={node} x={x} y={y} name={names.get(node.id) ?? node.id} kind={labels.kinds[node.kind]}
            current={node.id === selection.current} chosen={node.id !== selection.current && selection.others.includes(node.id)} lit={litOf(node.id)}
            keyed={hasKeys(node) ? colorLabels.keyframesPanel.keyed : null} passed={node.pass_through ? labels.passedThrough : null}
            viewed={node.id === viewed ? labels.viewedMark : null} ports={portNames} onLabel={startLabel} />)}
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
    {/* Select Tool: the kind taken is added as its button of the tool bar adds it, where the menu that opened the dialog was opened. */}
    {tool && !locked && <SelectTool onAdd={kind => add(kind, tool.pos, tool.after)} onClose={closeTool} />}
    {menu && <CoveringMenu menu={menu} onClose={closeMenu} />}
  </div>;
}
