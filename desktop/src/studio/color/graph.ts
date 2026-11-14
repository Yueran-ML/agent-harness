/**
 * A grade laid out as the node editor of DaVinci Resolve draws it (Reference Manual 20,
 * pp. 3309–3313): the source on the left, the output on the right, each node a card with its
 * number, its RGB input and output at the top of its sides and its key input and output at the
 * foot (a mixer's inputs down its left side), and the links between them.
 *
 * A node that the project gives a place (its `pos`, where it was dragged to, p. 3321) stands there;
 * the editor places the others: each node one column right of the furthest node that feeds it, the
 * nodes of a column in the order they were added (`Placed` holds the places given). The ports,
 * the halves of links and the nodes under a point are found here too, for the editor to draw
 * links and choose nodes by. Pure.
 */
import { sortLinks, type Grade, type GradeNode, type Link, type LinkKind } from '../engine';

export const NODE_WIDTH = 92;
export const NODE_HEIGHT = 58;
/** The room between two columns, where the links run, and between two rows. */
export const COLUMN_GAP = 56;
export const ROW_GAP = 34;
/** The margin round the graph, with room above a node for its label. */
export const MARGIN = 28;
/** The source and the output: a small round end at each side. */
export const END_RADIUS = 9;
/** How far the port of the picture lies below the top of a node, and the key's above its foot. */
export const PORT_INSET = 14;
/** How many characters of a label the editor shows at its size (p. 3313). */
export const LABEL_CHARACTERS = 12;

/** A node where the editor places it, with its number: its place in the order of the grade, from 1 (p. 3318). */
export interface PlacedNode {
  node: GradeNode;
  number: number;
  column: number;
  row: number;
  x: number;
  y: number;
}

/** A grade as the editor draws it. */
export interface Layout {
  nodes: PlacedNode[];
  /** The links in the engine's order, which is also the order they are drawn in. */
  links: Link[];
  source: { x: number; y: number };
  output: { x: number; y: number };
  width: number;
  height: number;
}

/** A node's number as the editor writes it: 01, 02 … */
export const numberText = (number: number): string => String(number).padStart(2, '0');

/** A label as the node shows it: the first characters, and an ellipsis where there are more. */
export function shownLabel(label: string): string {
  const characters = [...label];
  return characters.length <= LABEL_CHARACTERS ? label : `${characters.slice(0, LABEL_CHARACTERS).join('')}…`;
}

/** How many inputs of a kind a node has, as the engine counts them (`NodeKind::inputs`). */
export function inputsOf(node: GradeNode, kind: LinkKind): number {
  switch (node.kind) {
    case 'corrector': return 1;
    case 'parallel_mixer':
    case 'layer_mixer': return kind === 'rgb' ? node.params.inputs : 0;
    case 'key_mixer': return kind === 'key' ? node.params.inputs.length : 0;
    default: return 0;
  }
}

/** Whether a node has an output of a kind (`NodeKind::has_output`). */
export function hasOutput(node: GradeNode, kind: LinkKind): boolean {
  if (node.kind === 'corrector') return true;
  if (node.kind === 'key_mixer') return kind === 'key';
  return kind === 'rgb';
}

/**
 * Whether the grade colours the picture, as the engine decides it (`Grade::is_complete`): the output
 * takes a link, and every node takes a link into each of its inputs of the picture (of the key,
 * for a key mixer) and gives its picture (its key) to another. Otherwise DaVinci Resolve disables
 * the whole grade (pp. 3312, 3319), and so does the engine.
 */
export function isComplete(grade: Grade): boolean {
  const into = (to: string, kind: LinkKind, input: number) => grade.links.some(link => link.to === to && link.kind === kind && link.input === input);
  if (!into('output', 'rgb', 0)) return false;
  return grade.nodes.every((node) => {
    const main: LinkKind = node.kind === 'key_mixer' ? 'key' : 'rgb';
    for (let input = 0; input < inputsOf(node, main); input++) if (!into(node.id, main, input)) return false;
    return grade.links.some(link => link.from === node.id && link.kind === main);
  });
}

/**
 * The column of each node: one more than the furthest of those that feed it, the source being
 * column 0; a node that nothing feeds is in column 1. Links that loop (which the engine refuses)
 * are passed over.
 */
function columnsOf(grade: Grade): Map<string, number> {
  const columns = new Map<string, number>();
  const ids = new Set(grade.nodes.map(node => node.id));
  const feeding = new Map<string, string[]>(grade.nodes.map(node => [node.id, []]));
  for (const link of grade.links) if (ids.has(link.to) && (link.from === 'source' || ids.has(link.from))) feeding.get(link.to)!.push(link.from);
  const visiting = new Set<string>();
  const columnOf = (id: string): number => {
    if (id === 'source') return 0;
    const known = columns.get(id);
    if (known !== undefined) return known;
    if (visiting.has(id)) return 0;
    visiting.add(id);
    const column = 1 + Math.max(0, ...feeding.get(id)!.map(columnOf));
    visiting.delete(id);
    columns.set(id, column);
    return column;
  };
  for (const node of grade.nodes) columnOf(node.id);
  return columns;
}

/** A point of the graph, in its own units (a pixel at zoom 1). */
export interface Point { x: number; y: number }
/**
 * Where nodes lie, by their ids: the `pos` that the project keeps for them, and while a drag goes on
 * where it takes them (see `layoutGrade`). A node that is not in it stands where the editor places it.
 */
export type Placed = Readonly<Record<string, Point>>;
/** The least room above a node that is dragged up, for its label. */
const TOP = 14;

/**
 * Lay out a grade: where each node, the source and the output lie, and how large the whole is.
 * A node that was dragged (`placed`) stands where it was put; the others where the editor puts
 * them. Cleanup Node Graph (p. 3321) is laying it out again with none dragged.
 */
export function layoutGrade(grade: Grade, placed: Placed = {}): Layout {
  const columns = columnsOf(grade);
  const rowsUsed = new Map<number, number>();
  const nodes = grade.nodes.map((node, index) => {
    const column = columns.get(node.id) ?? 1;
    const row = rowsUsed.get(column) ?? 0;
    rowsUsed.set(column, row + 1);
    return { node, number: index + 1, column, row, x: 0, y: 0 };
  });
  const lastColumn = Math.max(0, ...nodes.map(each => each.column));
  const rows = Math.max(1, ...rowsUsed.values());
  // Column 0 is the source, a narrow one; then a column for each depth of nodes.
  const sourceX = MARGIN + END_RADIUS;
  const columnX = (column: number) => MARGIN + END_RADIUS * 2 + COLUMN_GAP + (column - 1) * (NODE_WIDTH + COLUMN_GAP);
  const rowY = (row: number) => MARGIN + row * (NODE_HEIGHT + ROW_GAP);
  for (const each of nodes) {
    const put = placed[each.node.id];
    each.x = put ? Math.max(0, put.x) : columnX(each.column);
    each.y = put ? Math.max(TOP, put.y) : rowY(each.row);
  }
  const autoOutputX = lastColumn === 0 ? sourceX + COLUMN_GAP * 2 + END_RADIUS * 2 : columnX(lastColumn) + NODE_WIDTH + COLUMN_GAP + END_RADIUS;
  // The output stays right of every node, one dragged far right too.
  const outputX = Math.max(autoOutputX, ...nodes.map(each => each.x + NODE_WIDTH + COLUMN_GAP / 2 + END_RADIUS));
  // The two ends are level with the first port of the picture of the first row.
  const endY = MARGIN + PORT_INSET;
  return {
    nodes,
    links: sortLinks(grade.links),
    source: { x: sourceX, y: endY },
    output: { x: outputX, y: endY },
    width: outputX + END_RADIUS + MARGIN,
    height: Math.max(rowY(rows - 1) + NODE_HEIGHT + MARGIN, ...nodes.map(each => each.y + NODE_HEIGHT + MARGIN)),
  };
}

/** Where an input of a node is: a corrector's picture at the top of its left side, its key at the foot; a mixer's spread down its side. */
export function inputPoint(placed: PlacedNode, kind: LinkKind, input: number): { x: number; y: number } {
  const { node, x, y } = placed;
  if (node.kind === 'corrector') return { x, y: kind === 'rgb' ? y + PORT_INSET : y + NODE_HEIGHT - PORT_INSET };
  const count = Math.max(1, inputsOf(node, kind));
  return { x, y: y + (NODE_HEIGHT * (input + 1)) / (count + 1) };
}

/** Where an output of a node is: the picture at the top of its right side, the key at the foot; a mixer's in the middle. */
export function outputPoint(placed: PlacedNode, kind: LinkKind): { x: number; y: number } {
  const { node, x, y } = placed;
  if (node.kind === 'corrector') return { x: x + NODE_WIDTH, y: kind === 'rgb' ? y + PORT_INSET : y + NODE_HEIGHT - PORT_INSET };
  return { x: x + NODE_WIDTH, y: y + NODE_HEIGHT / 2 };
}

/** The two ends of a link, where both are drawn; null for a link to or from a node that is not there. */
export function linkEnds(layout: Layout, link: Link): { from: { x: number; y: number }; to: { x: number; y: number } } | null {
  const byId = new Map(layout.nodes.map(each => [each.node.id, each]));
  const fromNode = byId.get(link.from);
  const toNode = byId.get(link.to);
  const from = link.from === 'source' ? { x: layout.source.x + END_RADIUS, y: layout.source.y } : fromNode ? outputPoint(fromNode, link.kind) : null;
  const to = link.to === 'output' ? { x: layout.output.x - END_RADIUS, y: layout.output.y } : toNode ? inputPoint(toNode, link.kind, link.input) : null;
  return from && to ? { from, to } : null;
}

/** The path of a link: a curve that leaves its output to the right and comes into its input from the left. */
export function linkPath(from: { x: number; y: number }, to: { x: number; y: number }): string {
  const bend = Math.max(24, Math.abs(to.x - from.x) / 2);
  return `M ${from.x} ${from.y} C ${from.x + bend} ${from.y}, ${to.x - bend} ${to.y}, ${to.x} ${to.y}`;
}

/** A link as one string, to name it in the page: `from>to:kind:input`. */
export const linkName = (link: Link): string => `${link.from}>${link.to}:${link.kind}:${link.input}`;

/** The four points of the curve of a link, as `linkPath` draws it. */
function controls(from: Point, to: Point): [Point, Point, Point, Point] {
  const bend = Math.max(24, Math.abs(to.x - from.x) / 2);
  return [from, { x: from.x + bend, y: from.y }, { x: to.x - bend, y: to.y }, to];
}
const mid = (a: Point, b: Point): Point => ({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });
const svgPoint = (point: Point) => `${point.x} ${point.y}`;

/**
 * The two halves of a link, as paths: the first, from its output, which selects the link when
 * clicked; and the second, into its input, which deletes it when clicked and takes its end along
 * when dragged (Reference Manual 20, p. 3319). Split at the middle of the curve of `linkPath`.
 */
export function linkHalves(from: Point, to: Point): [string, string] {
  const [p0, p1, p2, p3] = controls(from, to);
  const p01 = mid(p0, p1); const p12 = mid(p1, p2); const p23 = mid(p2, p3);
  const p012 = mid(p01, p12); const p123 = mid(p12, p23);
  const middle = mid(p012, p123);
  return [
    `M ${svgPoint(p0)} C ${svgPoint(p01)}, ${svgPoint(p012)}, ${svgPoint(middle)}`,
    `M ${svgPoint(middle)} C ${svgPoint(p123)}, ${svgPoint(p23)}, ${svgPoint(p3)}`,
  ];
}

/** The point of the curve of a link at `t`, from 0 at its output to 1 at its input. */
export function pointOnLink(from: Point, to: Point, t: number): Point {
  const [p0, p1, p2, p3] = controls(from, to);
  const u = 1 - t;
  const along = (a: number, b: number, c: number, d: number) => u * u * u * a + 3 * u * u * t * b + 3 * u * t * t * c + t * t * t * d;
  return { x: along(p0.x, p1.x, p2.x, p3.x), y: along(p0.y, p1.y, p2.y, p3.y) };
}

/** A port of the graph: an input or an output of a kind of a node, the source (an output) or the output (an input). */
export interface Port { node: string; side: 'in' | 'out'; kind: LinkKind; input: number }
/** How near a port the pointer must be to take it, in units of the graph. */
export const PORT_REACH = 10;
const distance = (a: Point, b: Point) => Math.hypot(a.x - b.x, a.y - b.y);

/** Every port of a layout and where it is. */
export function portsOf(layout: Layout): { port: Port; at: Point }[] {
  const ports: { port: Port; at: Point }[] = [
    { port: { node: 'source', side: 'out', kind: 'rgb', input: 0 }, at: { x: layout.source.x + END_RADIUS, y: layout.source.y } },
    { port: { node: 'output', side: 'in', kind: 'rgb', input: 0 }, at: { x: layout.output.x - END_RADIUS, y: layout.output.y } },
  ];
  for (const placed of layout.nodes) {
    for (const kind of ['rgb', 'key'] as const) {
      for (let input = 0; input < inputsOf(placed.node, kind); input++) {
        ports.push({ port: { node: placed.node.id, side: 'in', kind, input }, at: inputPoint(placed, kind, input) });
      }
      if (hasOutput(placed.node, kind)) ports.push({ port: { node: placed.node.id, side: 'out', kind, input: 0 }, at: outputPoint(placed, kind) });
    }
  }
  return ports;
}

/** The port nearest `point` within reach that `accepts`, or null. */
export function portAt(layout: Layout, point: Point, accepts: (port: Port) => boolean = () => true): Port | null {
  let best: { port: Port; away: number } | null = null;
  for (const { port, at } of portsOf(layout)) {
    const away = distance(at, point);
    if (away <= PORT_REACH && accepts(port) && (best === null || away < best.away)) best = { port, away };
  }
  return best?.port ?? null;
}

/** The node whose card lies under `point`, the one drawn last where cards lie on one another. */
export function nodeAt(layout: Layout, point: Point): PlacedNode | null {
  for (let at = layout.nodes.length - 1; at >= 0; at--) {
    const each = layout.nodes[at];
    if (point.x >= each.x && point.x <= each.x + NODE_WIDTH && point.y >= each.y && point.y <= each.y + NODE_HEIGHT) return each;
  }
  return null;
}

/** The nodes whose cards a box drawn from `a` to `b` touches (p. 3314). */
export function nodesIn(layout: Layout, a: Point, b: Point): string[] {
  const left = Math.min(a.x, b.x); const right = Math.max(a.x, b.x);
  const top = Math.min(a.y, b.y); const bottom = Math.max(a.y, b.y);
  return layout.nodes.filter(each => each.x <= right && each.x + NODE_WIDTH >= left && each.y <= bottom && each.y + NODE_HEIGHT >= top)
    .map(each => each.node.id);
}

/** How near a link the pointer must be for a node dropped there to go into it (p. 3319). */
export const LINK_REACH = 8;

/** The link whose curve passes within reach of `point`, or null. */
export function linkNear(layout: Layout, point: Point): Link | null {
  let best: { link: Link; away: number } | null = null;
  for (const link of layout.links) {
    const ends = linkEnds(layout, link);
    if (!ends) continue;
    for (let step = 0; step <= 32; step++) {
      const away = distance(pointOnLink(ends.from, ends.to, step / 32), point);
      if (away <= LINK_REACH && (best === null || away < best.away)) best = { link, away };
    }
  }
  return best?.link ?? null;
}
