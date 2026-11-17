/**
 * A composition laid out as the node editor of the Fusion page draws it (DaVinci Resolve Reference
 * Manual 20, Ch. 67, pp. 1316–1365): each node a card with the inputs of its pictures down its left
 * side, its mask input on its lower edge, the two matte inputs of a Delta Keyer on its upper edge
 * and its one output at the middle of its right side, and the links between them, from left to right.
 *
 * A node that the project gives a place (its `pos`, where it was dragged to) stands there; the
 * editor places the others: a node that nothing feeds in the first column, every other one column
 * right of the furthest node that feeds it, the nodes of a column in the order they were added.
 * The ports, the links and the nodes under a point are found here too. Pure, and built on the
 * measures and the curve of a link that the Color page's node editor has (`color/graph.ts`), so
 * that the two editors are drawn alike.
 */
import { sortFusionLinks, type Composition, type FusionInput, type FusionKind, type FusionLink, type FusionNode } from '../engine';
import {
  COLUMN_GAP, LINK_REACH, linkHalves, linkPath, MARGIN, NODE_HEIGHT, NODE_WIDTH, pointOnLink, PORT_REACH, ROW_GAP, type Placed, type Point,
} from '../color/graph';

/**
 * The inputs of each kind of node, the first first, as the engine names them (PROTOCOL.md,
 * "Compositions"): the pictures of a node, and after them its `mask`, the effect mask of a Merge, a
 * Transform, a Background, a Blur, a Mosaic, a Delta Keyer and a Tracker and the mask that comes
 * into a mask node. A Delta Keyer has its two mattes between its picture and its mask; a Tracker
 * has two pictures, the one that it tracks (`input`), which is the one that it hands on, and the
 * `foreground` that it makes follow its patterns.
 */
export const INPUTS: Record<FusionKind, readonly FusionInput[]> = {
  media_in: [], media_out: ['input'], merge: ['background', 'foreground', 'mask'], transform: ['input', 'mask'], background: ['mask'],
  rectangle: ['mask'], ellipse: ['mask'], polygon: ['mask'],
  blur: ['input', 'mask'], mosaic: ['input', 'mask'],
  delta_keyer: ['input', 'garbage_matte', 'solid_matte', 'mask'],
  tracker: ['input', 'foreground', 'mask'],
};

/** The kinds of node that make a mask (PROTOCOL.md, "Masks"). */
export const MASK_KINDS = ['rectangle', 'ellipse', 'polygon'] as const;
export type MaskKind = (typeof MASK_KINDS)[number];
/** Whether a kind of node makes a mask. */
export const isMaskKind = (kind: FusionKind): kind is MaskKind => kind === 'rectangle' || kind === 'ellipse' || kind === 'polygon';

/** Whether a kind of node has a `mask` input. */
export const hasMaskInput = (kind: FusionKind): boolean => INPUTS[kind].includes('mask');

/**
 * The inputs that take a matte (PROTOCOL.md, "The Delta Keyer"): the garbage matte and the solid
 * matte of a Delta Keyer. As its effect mask, a matte is no picture that its node hands on.
 */
export const MATTE_INPUTS = ['garbage_matte', 'solid_matte'] as const;
export type MatteInput = (typeof MATTE_INPUTS)[number];
/** Whether an input takes a matte. */
export const isMatteInput = (input: FusionInput | null): input is MatteInput => input === 'garbage_matte' || input === 'solid_matte';
/** Whether an input takes a picture: every input but a `mask` and a matte. */
export const isPictureInput = (input: FusionInput): boolean => input !== 'mask' && !isMatteInput(input);

/** The inputs of a kind of node that are drawn down its left side: those of its pictures, which are all but its `mask` and its mattes. */
export const pictureInputs = (kind: FusionKind): readonly FusionInput[] => INPUTS[kind].filter(isPictureInput);
/** The inputs of a kind of node that take a matte, which are drawn on its upper edge: those of a Delta Keyer, and of no other kind. */
export const matteInputs = (kind: FusionKind): readonly MatteInput[] => INPUTS[kind].filter(isMatteInput);

/**
 * The input whose picture a node hands on where it is passed through or taken away: the first of its
 * pictures (the `background` of a Merge, the `input` of a Transform, of a Delta Keyer and of a
 * Tracker), of a mask node the mask that comes into it, and none of a Background or a MediaIn, whose
 * effect mask is no picture to hand on.
 */
export const handedOn = (kind: FusionKind): FusionInput | undefined => pictureInputs(kind)[0] ?? (isMaskKind(kind) ? 'mask' : undefined);

/** How far from the left edge of a node its mask input lies, on the lower edge: clear of the badge of keys at the middle. */
export const MASK_INSET = NODE_WIDTH / 4;
/**
 * How far from the left edge of a node each of its matte inputs lies, on the upper edge: the garbage
 * matte a quarter of the way along, over the mask input, and the solid matte three quarters.
 */
export const MATTE_INSETS: Record<MatteInput, number> = { garbage_matte: MASK_INSET, solid_matte: NODE_WIDTH - MASK_INSET };

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

/**
 * Where an input of a node is: those of its pictures spread down its left side, the first at the top
 * (a Merge's background above its foreground); its `mask` on its lower edge, a quarter of the way
 * along, and the mattes of a Delta Keyer on its upper edge (`MATTE_INSETS`), wherever the others are.
 */
export function inputPoint(placed: PlacedFusion, input: FusionInput): Point {
  if (input === 'mask') return { x: placed.x + MASK_INSET, y: placed.y + NODE_HEIGHT };
  if (isMatteInput(input)) return { x: placed.x + MATTE_INSETS[input], y: placed.y };
  const inputs = pictureInputs(placed.node.kind);
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

/** The edge of a node that an input lies on where it is not on its left side: the lower one for a `mask`, the upper one for a matte. */
type Edge = 'below' | 'above';
const edgeOf = (input: FusionInput | null): Edge | null => (input === 'mask' ? 'below' : isMatteInput(input) ? 'above' : null);

/**
 * The four points of the curve of a link into a `mask` or a matte: it leaves its output to the
 * right, as every link does, and comes into the mask input from below, or into the matte input from
 * above, where that input lies.
 */
function edgeCurve(from: Point, to: Point, edge: Edge): [Point, Point, Point, Point] {
  const bend = Math.max(24, Math.abs(to.x - from.x) / 2);
  const rise = Math.max(24, Math.abs(to.y - from.y) / 2);
  return [from, { x: from.x + bend, y: from.y }, { x: to.x, y: edge === 'below' ? to.y + rise : to.y - rise }, to];
}
const between = (a: Point, b: Point): Point => ({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });
const written = (point: Point) => `${point.x} ${point.y}`;

/** The path of a link into an input: the curve of the Color page's editor, but into a `mask` from below and into a matte from above. */
export function fusionLinkPath(from: Point, to: Point, input: FusionInput | null): string {
  const edge = edgeOf(input);
  if (edge === null) return linkPath(from, to);
  const [p0, p1, p2, p3] = edgeCurve(from, to, edge);
  return `M ${written(p0)} C ${written(p1)}, ${written(p2)}, ${written(p3)}`;
}

/** The two halves of a link into an input, as paths, split at the middle of the curve of `fusionLinkPath` (see `linkHalves`). */
export function fusionLinkHalves(from: Point, to: Point, input: FusionInput | null): [string, string] {
  const edge = edgeOf(input);
  if (edge === null) return linkHalves(from, to);
  const [p0, p1, p2, p3] = edgeCurve(from, to, edge);
  const p01 = between(p0, p1); const p12 = between(p1, p2); const p23 = between(p2, p3);
  const p012 = between(p01, p12); const p123 = between(p12, p23);
  const middle = between(p012, p123);
  return [
    `M ${written(p0)} C ${written(p01)}, ${written(p012)}, ${written(middle)}`,
    `M ${written(middle)} C ${written(p123)}, ${written(p23)}, ${written(p3)}`,
  ];
}

/** The point of the curve of a link into an input at `t`, from 0 at its output to 1 at its input: the curve that `fusionLinkPath` draws. */
function pointOnFusionLink(from: Point, to: Point, input: FusionInput, t: number): Point {
  const edge = edgeOf(input);
  if (edge === null) return pointOnLink(from, to, t);
  const [p0, p1, p2, p3] = edgeCurve(from, to, edge);
  const u = 1 - t;
  const along = (a: number, b: number, c: number, d: number) => u * u * u * a + 3 * u * u * t * b + 3 * u * t * t * c + t * t * t * d;
  return { x: along(p0.x, p1.x, p2.x, p3.x), y: along(p0.y, p1.y, p2.y, p3.y) };
}

/**
 * The link that `accepts` whose curve passes nearest `point` within reach, or null: where a node
 * that is dragged over a link would go into it, as in the Color page's editor (`linkNear`, with its
 * reach).
 */
export function fusionLinkNear(layout: FusionLayout, point: Point, accepts: (link: FusionLink) => boolean = () => true): FusionLink | null {
  let best: { link: FusionLink; away: number } | null = null;
  for (const link of layout.links) {
    if (!accepts(link)) continue;
    const ends = linkEnds(layout, link);
    if (!ends) continue;
    for (let step = 0; step <= 32; step++) {
      const on = pointOnFusionLink(ends.from, ends.to, link.input, step / 32);
      const away = Math.hypot(on.x - point.x, on.y - point.y);
      if (away <= LINK_REACH && (best === null || away < best.away)) best = { link, away };
    }
  }
  return best?.link ?? null;
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
