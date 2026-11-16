/**
 * A composition laid out as the node editor of the Fusion page draws it (DaVinci Resolve Reference
 * Manual 20, Ch. 67, pp. 1316–1365): each node a card with its inputs down its left side and its
 * one output at the middle of its right side, and the links between them, from left to right.
 *
 * A node that the project gives a place (its `pos`, where it was dragged to) stands there; the
 * editor places the others: a node that nothing feeds in the first column, every other one column
 * right of the furthest node that feeds it, the nodes of a column in the order they were added.
 * The ports, the links and the nodes under a point are found here too. Pure, and built on the
 * measures and the curve of a link that the Color page's node editor has (`color/graph.ts`), so
 * that the two editors are drawn alike.
 */
import { sortFusionLinks, type Composition, type FusionInput, type FusionKind, type FusionLink, type FusionNode } from '../engine';
import { COLUMN_GAP, MARGIN, NODE_HEIGHT, NODE_WIDTH, PORT_REACH, ROW_GAP, type Placed, type Point } from '../color/graph';

/** The inputs of each kind of node, the first first, as the engine names them (PROTOCOL.md, "Compositions"). */
export const INPUTS: Record<FusionKind, readonly FusionInput[]> = {
  media_in: [], media_out: ['input'], merge: ['background', 'foreground'], transform: ['input'], background: [],
  // The mask that comes into a mask node. The effect mask of the other kinds (`mask`, after their pictures) is not drawn by the editor yet.
  rectangle: ['mask'], ellipse: ['mask'], polygon: ['mask'],
};

/** Whether a kind of node has an output: every kind but MediaOut, whose picture goes back to the timeline. */
export const hasOutput = (kind: FusionKind): boolean => kind !== 'media_out';

/** A node where the editor places it. */
export interface PlacedFusion { node: FusionNode; column: number; row: number; x: number; y: number }

/** A composition as the editor draws it. */
export interface FusionLayout {
  nodes: PlacedFusion[];
  /** The links in the engine's order, which is also the order they are drawn in. */
  links: FusionLink[];
  width: number;
  height: number;
}

/** The least room above a node that is dragged up, so that its card stays whole in the graph. */
const TOP = 4;

/**
 * The column of each node: 0 for one that nothing feeds, else one more than the furthest of those
 * that feed it. Links that loop (which the engine refuses) are passed over.
 */
function columnsOf(composition: Composition): Map<string, number> {
  const columns = new Map<string, number>();
  const feeding = new Map<string, string[]>(composition.nodes.map(node => [node.id, []]));
  for (const link of composition.links) if (feeding.has(link.to) && feeding.has(link.from)) feeding.get(link.to)!.push(link.from);
  const visiting = new Set<string>();
  const columnOf = (id: string): number => {
    const known = columns.get(id);
    if (known !== undefined) return known;
    if (visiting.has(id)) return 0;
    visiting.add(id);
    const feeders = feeding.get(id)!;
    const column = feeders.length === 0 ? 0 : 1 + Math.max(...feeders.map(columnOf));
    visiting.delete(id);
    columns.set(id, column);
    return column;
  };
  for (const node of composition.nodes) columnOf(node.id);
  return columns;
}

/** Where the nodes of a composition lie that the project gives a place (`pos`); the others the editor lays out. */
export function placesOf(composition: Composition): Placed {
  return Object.fromEntries(composition.nodes.flatMap(node => (node.pos ? [[node.id, node.pos]] : [])));
}

/**
 * Lay out a composition: where each node lies, and how large the whole is. A node that was dragged
 * (`placed`) stands where it was put; the others where the editor puts them.
 */
export function layoutComposition(composition: Composition, placed: Placed = {}): FusionLayout {
  const columns = columnsOf(composition);
  const rowsUsed = new Map<number, number>();
  const nodes = composition.nodes.map((node): PlacedFusion => {
    const column = columns.get(node.id) ?? 0;
    const row = rowsUsed.get(column) ?? 0;
    rowsUsed.set(column, row + 1);
    const put = placed[node.id];
    return {
      node, column, row,
      x: put ? Math.max(0, put.x) : MARGIN + column * (NODE_WIDTH + COLUMN_GAP),
      y: put ? Math.max(TOP, put.y) : MARGIN + row * (NODE_HEIGHT + ROW_GAP),
    };
  });
  return {
    nodes,
    links: sortFusionLinks(composition.links),
    width: Math.max(MARGIN * 2 + NODE_WIDTH, ...nodes.map(each => each.x + NODE_WIDTH + MARGIN)),
    height: Math.max(MARGIN * 2 + NODE_HEIGHT, ...nodes.map(each => each.y + NODE_HEIGHT + MARGIN)),
  };
}

/** Where an input of a node is: spread down its left side, the first at the top (a Merge's background above its foreground). */
export function inputPoint(placed: PlacedFusion, input: FusionInput): Point {
  const inputs = INPUTS[placed.node.kind];
  const at = Math.max(0, inputs.indexOf(input));
  return { x: placed.x, y: placed.y + (NODE_HEIGHT * (at + 1)) / (Math.max(1, inputs.length) + 1) };
}

/** Where the output of a node is: the middle of its right side. */
export function outputPoint(placed: PlacedFusion): Point {
  return { x: placed.x + NODE_WIDTH, y: placed.y + NODE_HEIGHT / 2 };
}

/** The two ends of a link, where both its nodes are drawn; null for a link to or from a node that is not there. */
export function linkEnds(layout: FusionLayout, link: FusionLink): { from: Point; to: Point } | null {
  const fromNode = layout.nodes.find(each => each.node.id === link.from);
  const toNode = layout.nodes.find(each => each.node.id === link.to);
  return fromNode && toNode ? { from: outputPoint(fromNode), to: inputPoint(toNode, link.input) } : null;
}

/** A link as one string, to name it in the page: `from>to:input`. */
export const linkName = (link: FusionLink): string => `${link.from}>${link.to}:${link.input}`;

/** A port of the graph: an input of a node by its name, or its output. */
export interface FusionPort { node: string; side: 'in' | 'out'; input: FusionInput | null }

/** A port as the page names it on the element that draws it: `in:background`, `out`. */
export const portName = (port: Pick<FusionPort, 'side' | 'input'>): string => (port.side === 'in' ? `in:${port.input}` : 'out');

/** Every port of a layout and where it is. */
export function portsOf(layout: FusionLayout): { port: FusionPort; at: Point }[] {
  return layout.nodes.flatMap((placed) => {
    const ports: { port: FusionPort; at: Point }[] = INPUTS[placed.node.kind]
      .map(input => ({ port: { node: placed.node.id, side: 'in' as const, input }, at: inputPoint(placed, input) }));
    if (hasOutput(placed.node.kind)) ports.push({ port: { node: placed.node.id, side: 'out', input: null }, at: outputPoint(placed) });
    return ports;
  });
}

/** The port nearest `point` within reach that `accepts`, or null. */
export function portAt(layout: FusionLayout, point: Point, accepts: (port: FusionPort) => boolean = () => true): FusionPort | null {
  let best: { port: FusionPort; away: number } | null = null;
  for (const { port, at } of portsOf(layout)) {
    const away = Math.hypot(at.x - point.x, at.y - point.y);
    if (away <= PORT_REACH && accepts(port) && (best === null || away < best.away)) best = { port, away };
  }
  return best?.port ?? null;
}

/** Where a port is drawn, in a layout; null for a port of a node that is not there. */
export function portPoint(layout: FusionLayout, port: FusionPort): Point | null {
  const placed = layout.nodes.find(each => each.node.id === port.node);
  if (!placed) return null;
  return port.side === 'in' && port.input !== null ? inputPoint(placed, port.input) : outputPoint(placed);
}

/** The nodes whose cards a box drawn from `a` to `b` touches. */
export function nodesIn(layout: FusionLayout, a: Point, b: Point): string[] {
  const left = Math.min(a.x, b.x); const right = Math.max(a.x, b.x);
  const top = Math.min(a.y, b.y); const bottom = Math.max(a.y, b.y);
  return layout.nodes.filter(each => each.x <= right && each.x + NODE_WIDTH >= left && each.y <= bottom && each.y + NODE_HEIGHT >= top)
    .map(each => each.node.id);
}

/**
 * The name that each node is shown by: its label where it has one, else the name of its kind and its
 * number among the nodes of that kind, in the order they were added (Fusion's `Merge1`, `Merge2`).
 */
export function namesOf(composition: Composition, kinds: Record<FusionKind, string>): Map<string, string> {
  const counted = new Map<FusionKind, number>();
  return new Map(composition.nodes.map((node) => {
    const number = (counted.get(node.kind) ?? 0) + 1;
    counted.set(node.kind, number);
    return [node.id, node.label || `${kinds[node.kind]} ${number}`];
  }));
}
