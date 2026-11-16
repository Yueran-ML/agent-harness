/**
 * What the node editor of the Fusion page asks of the engine (PROTOCOL.md, "Compositions"; DaVinci
 * Resolve Reference Manual 20, Ch. 67): the operations of each command, and the keys that ask for
 * them. Pure, so that the editor, the page and the tests work from the same rules.
 *
 * A clip that has no composition is shown with the one that DaVinci gives a clip when the Fusion
 * page is first opened on it (`firstComposition`, a MediaIn into a MediaOut), which draws the clip
 * as it is. It is not in the project until something is changed: the first change sends
 * `set_fusion` with it and then the change, in one transaction (`withFirst`), so that looking at a
 * clip on the page changes nothing and one undo takes the change and the composition back.
 */
import type { Key } from '../../app/keys';
import {
  firstComposition, sortFusionLinks, type Clip, type Composition, type FusionBackground, type FusionBlur, type FusionEllipse, type FusionKind, type FusionLink,
  type FusionMaskCommon, type FusionMerge, type FusionMosaic, type FusionNode, type FusionPoint, type FusionPolygon, type FusionRectangle, type FusionTransform,
  type NodePos, type Op,
} from '../engine';
import { valueAt } from '../keyframes/value';
import { handedOn, hasMaskInput, hasOutput, isMaskKind, MASK_KINDS, pictureInputs, type MaskKind } from './graph';

/** The clip whose composition the operations change, as they name it. */
export interface FusionAt { timeline: string; clip: string }

/** The composition that a clip without one is shown with: the same object each time, so that what is drawn from it is not drawn again. */
const FIRST: Composition = firstComposition();

/** The composition of a clip as the page shows it: its own, or the first one where it has none. */
export function shownComposition(clip: Pick<Clip, 'fusion'>): Composition {
  return clip.fusion ?? FIRST;
}

/**
 * The operations of a change as they are sent: where the clip has no composition yet (`has` false),
 * `set_fusion` with the first one goes before them. A change that puts a whole composition in place
 * itself, or that is nothing, is sent as it is.
 */
export function withFirst(at: FusionAt, has: boolean, ops: Op[]): Op[] {
  if (has || ops.length === 0 || ops[0].op === 'set_fusion') return ops;
  return [{ op: 'set_fusion', ...at, value: firstComposition() }, ...ops];
}

/** The kinds of node of a picture that the editor adds: a composition has exactly one MediaOut, so none is added. */
export const ADDABLE: readonly Exclude<FusionKind, 'media_out'>[] = ['merge', 'transform', 'background', 'media_in'];
/** The masks that the editor adds, in a group of their own. */
export const ADDABLE_MASKS: readonly MaskKind[] = MASK_KINDS;

/** The settings of a Merge as the engine makes a new one (PROTOCOL.md, "Compositions"). */
export const newMerge = (): FusionMerge => ({
  apply: 'normal', operator: 'over', subtractive_additive: 1, alpha_gain: 1, burn_in: 0, blend: 1, center: { x: 0.5, y: 0.5 }, size: 1, angle: 0,
});
/** The settings of a Transform as the engine makes a new one. */
export const newTransform = (): FusionTransform => ({ center: { x: 0.5, y: 0.5 }, size: 1, angle: 0, flip_horizontal: false, flip_vertical: false });
/** The settings of a Background as the engine makes a new one: opaque black. */
export const newBackground = (): FusionBackground => ({ color: { r: 0, g: 0, b: 0, a: 1 } });
/** The controls that every mask has, as the engine makes a new one: solid and whole, with a crisp edge and no border, not inverted. */
const newMask = (): FusionMaskCommon => ({ level: 1, soft_edge: 0, border_width: 0, invert: false, solid: true, paint_mode: 'merge' });
/** The settings of a Rectangle mask as the engine makes a new one: in the middle, half the picture wide and high. */
export const newRectangle = (): FusionRectangle => ({ ...newMask(), center: { x: 0.5, y: 0.5 }, width: 0.5, height: 0.5, corner_radius: 0, angle: 0 });
/** The settings of an Ellipse mask as the engine makes a new one: a circle in the middle, half the picture wide. */
export const newEllipse = (): FusionEllipse => ({ ...newMask(), center: { x: 0.5, y: 0.5 }, width: 0.5, height: 0.5, angle: 0 });
/** The settings of a Polygon mask as the engine makes a new one: no points yet. */
export const newPolygon = (): FusionPolygon => ({ ...newMask(), center: { x: 0.5, y: 0.5 }, size: 1, angle: 0, points: [], fill: 'alternate' });

/** The settings of a Blur as the engine makes a new one (PROTOCOL.md, "Blur and Mosaic"): a Gaussian of 5, across and down alike, shown whole. */
export const newBlur = (): FusionBlur => ({ filter: 'gaussian', lock_xy: true, size: { x: 5, y: 5 }, blend: 1 });
/** The settings of a Mosaic as the engine makes a new one: 32 square cells across, the grid where it lies. */
export const newMosaic = (): FusionMosaic => ({ frequency: 32, aspect: 1, offset: { x: 0, y: 0 }, whole_cells_only: false });

/** The most points that a Polygon may have (PROTOCOL.md, "Masks"). */
export const MOST_POINTS = 128;
/** The fewest points that make a shape: with fewer a Polygon is a mask of nothing, so the page takes none away below them. */
export const LEAST_POINTS = 3;
/**
 * The points that a Polygon added on the page begins with, since its points cannot be drawn in the
 * viewer yet: a square about its centre, four tenths of the width of the picture wide, from its
 * lower left corner round counter-clockwise (a point is counted from the centre, `x` to the right
 * and `y` up, both in widths of the picture).
 */
export const startPoints = (): FusionPoint[] => [{ x: -0.2, y: -0.2 }, { x: 0.2, y: -0.2 }, { x: 0.2, y: 0.2 }, { x: -0.2, y: 0.2 }];

/** A place as the editor keeps it: to a whole pixel, which is as near as a pointer puts a node. */
const placeAt = (at: NodePos): NodePos => ({ x: Math.round(at.x), y: Math.round(at.y) });

/** A node of a kind with every setting at its default, for the engine to give an id (`id` empty). */
export function newNode(kind: FusionKind, pos: NodePos | null = null, id = ''): FusionNode {
  const base = { id, label: '', pass_through: false, pos: pos ? placeAt(pos) : null };
  switch (kind) {
    case 'merge': return { ...base, kind, params: newMerge() };
    case 'transform': return { ...base, kind, params: newTransform() };
    case 'background': return { ...base, kind, params: newBackground() };
    case 'rectangle': return { ...base, kind, params: newRectangle() };
    case 'ellipse': return { ...base, kind, params: newEllipse() };
    case 'polygon': return { ...base, kind, params: newPolygon() };
    case 'blur': return { ...base, kind, params: newBlur() };
    case 'mosaic': return { ...base, kind, params: newMosaic() };
    default: return { ...base, kind, params: {} };
  }
}

/**
 * Add a node. With `after`, a node that has an output, a node that has an input of a picture is
 * inserted after it: the links of the operation put the output of `after` into the first input of
 * the new node (`input` of a Transform, `background` of a Merge), and the output of the new node
 * into every input that `after` fed, each in the place of the link that was there. A node without
 * such an input (a Background, a MediaIn), or one added with no node chosen, is added with no links.
 *
 * A mask (a Rectangle, an Ellipse, a Polygon) is not put into the chain of the picture: as in
 * Fusion, where the node chosen has a `mask` input that nothing is linked into, the mask is linked
 * into it, in the same operation, and else it is added with no links. A Polygon begins with the
 * points of `startPoints`.
 *
 * The new node is named by the empty id in its links, as `add_fusion_node` takes them.
 */
export function addNode(at: FusionAt, composition: Composition, kind: FusionKind, after: string | null, pos: NodePos | null): Op[] {
  const node: FusionNode = kind === 'polygon'
    ? { id: '', label: '', pass_through: false, pos: pos ? placeAt(pos) : null, kind, params: { ...newPolygon(), points: startPoints() } }
    : newNode(kind, pos);
  const before = after === null ? undefined : composition.nodes.find(each => each.id === after);
  if (isMaskKind(kind)) {
    const into = before === undefined ? null : before.id;
    const free = before !== undefined && hasMaskInput(before.kind) && !composition.links.some(link => link.to === into && link.input === 'mask');
    if (into === null || !free) return [{ op: 'add_fusion_node', ...at, node }];
    return [{ op: 'add_fusion_node', ...at, node, links: [{ from: '', to: into, input: 'mask' }] }];
  }
  const first = pictureInputs(kind)[0];
  if (!before || !hasOutput(before.kind) || first === undefined) return [{ op: 'add_fusion_node', ...at, node }];
  const fed = sortFusionLinks(composition.links.filter(link => link.from === before.id));
  const links: FusionLink[] = [{ from: before.id, to: '', input: first }, ...fed.map(link => ({ from: '', to: link.to, input: link.input }))];
  return [{ op: 'add_fusion_node', ...at, node, links }];
}

/** The id of the node that a transaction added, which becomes the one chosen. */
export function addedNode(done: readonly Op[] | null | undefined): string | null {
  const added = (done ?? []).filter((op): op is Extract<Op, { op: 'add_fusion_node' }> => op.op === 'add_fusion_node');
  return added.at(-1)?.node.id ?? null;
}

/**
 * Delete nodes. The MediaOut is left: a composition has exactly one, and the engine refuses to take
 * it away. As Fusion does, what fed a node is joined to what the node fed where that is a single
 * chain: a node with exactly one link into it gives each input that it fed a link from the node
 * that fed it (the `links` of `remove_fusion_node`). A node with no link into it, or with two (a
 * Merge with both inputs linked), leaves what it fed without a picture. The effect mask of a node
 * is not counted among the links into it, since it is no picture that the node hands on: a
 * Transform with a mask is joined across as one without, and a Background with a mask leaves what
 * it fed without a picture; the mask that comes into a mask node is the picture that it hands on,
 * and is counted. Each node is taken away from the composition as the ones before it left it, as
 * the engine applies them.
 */
export function removeNodes(at: FusionAt, composition: Composition, ids: readonly string[]): Op[] {
  let now = composition;
  const ops: Op[] = [];
  for (const id of ids) {
    const node = now.nodes.find(each => each.id === id);
    if (!node || node.kind === 'media_out') continue;
    const passed = handedOn(node.kind);
    const into = now.links.filter(link => link.to === id && (link.input !== 'mask' || passed === 'mask'));
    const fed = now.links.filter(link => link.from === id);
    const links: FusionLink[] = into.length === 1 ? fed.map(link => ({ from: into[0].from, to: link.to, input: link.input })) : [];
    ops.push(links.length > 0 ? { op: 'remove_fusion_node', ...at, id, links } : { op: 'remove_fusion_node', ...at, id });
    now = { nodes: now.nodes.filter(each => each.id !== id), links: sortFusionLinks([...now.links.filter(link => link.from !== id && link.to !== id), ...links]) };
  }
  return ops;
}

/**
 * Nodes at new places: one `set_fusion_node` of `pos` each. `all` sends every node given, which is
 * what a move of a drag sends (the same whatever went before it); without it a node put where it is
 * sends nothing.
 */
export function moveNodes(at: FusionAt, composition: Composition, moved: Readonly<Record<string, NodePos>>, all = false): Op[] {
  return composition.nodes.flatMap((node): Op[] => {
    const to = moved[node.id];
    if (to === undefined) return [];
    const pos = placeAt(to);
    return !all && node.pos?.x === pos.x && node.pos.y === pos.y ? [] : [{ op: 'set_fusion_node', ...at, id: node.id, pos }];
  });
}

/** The nodes put back where they were before a drag: each at its `pos`, or where the editor lays it out. */
export function restoreNodes(at: FusionAt, composition: Composition, ids: readonly string[]): Op[] {
  return composition.nodes.filter(node => ids.includes(node.id)).map((node): Op => ({ op: 'set_fusion_node', ...at, id: node.id, pos: node.pos ?? null }));
}

/** Link an output into an input: a link that the input had is put out of its place. The link that is there already sends nothing. */
export function connect(at: FusionAt, composition: Composition, link: FusionLink): Op[] {
  const there = composition.links.some(each => each.to === link.to && each.input === link.input && each.from === link.from);
  return there ? [] : [{ op: 'connect_fusion', ...at, link }];
}

/** Take a link away. */
export function disconnect(at: FusionAt, link: FusionLink): Op[] {
  return [{ op: 'disconnect_fusion', ...at, to: link.to, input: link.input }];
}

/** Take the end of a link to another input: the old link goes, the new one comes, in one transaction. */
export function reconnect(at: FusionAt, old: FusionLink, into: Pick<FusionLink, 'to' | 'input'>): Op[] {
  if (old.to === into.to && old.input === into.input) return [];
  return [...disconnect(at, old), { op: 'connect_fusion', ...at, link: { from: old.from, to: into.to, input: into.input } }];
}

/** Pass Through (Ctrl+P): each node chosen the other way. */
export function togglePass(at: FusionAt, composition: Composition, ids: readonly string[]): Op[] {
  return composition.nodes.filter(node => ids.includes(node.id)).map((node): Op => ({ op: 'set_fusion_node', ...at, id: node.id, pass_through: !node.pass_through }));
}

/** Change a node's label (F2); an empty one takes it away, and the node is named by its kind again. */
export function relabel(at: FusionAt, composition: Composition, id: string, label: string): Op[] {
  const node = composition.nodes.find(each => each.id === id);
  return !node || node.label === label ? [] : [{ op: 'set_fusion_node', ...at, id, label }];
}

/** A number as a new point is given it: to a millionth, so that the middle of two numbers is written as it is read. */
const tidy = (value: number): number => Number(value.toFixed(6));

/**
 * Put a point into a Polygon after its point `after` (counted from 0), or after its last where
 * `after` is null or names none. The list of points is set whole, as the engine takes it
 * (`set_fusion_node` with `points`), and every point that was there goes into it as the object that
 * it is, so that its numbers and its keys stay with it though its place in the list, by which its
 * keys are named, becomes another. The new point lies in the middle of the side that it is put
 * into, between the point before it and the one after it (the first, after the last: the outline
 * closes), where they are at the frame `keyFrame` of their keys; beside the one point of a polygon
 * that has one; and at the centre of one that has none. Nothing where the polygon is full
 * (`MOST_POINTS`), and for a node that is no Polygon.
 */
export function insertPoint(at: FusionAt, node: FusionNode, after: number | null, keyFrame: number): Op[] {
  if (node.kind !== 'polygon') return [];
  const points = node.params.points;
  if (points.length >= MOST_POINTS) return [];
  const index = after === null || !Number.isInteger(after) || after < 0 || after >= points.length ? points.length : after + 1;
  const before = points[index - 1];
  const next = points[index % Math.max(1, points.length)];
  const now = (point: FusionPoint) => ({ x: valueAt(point.x, keyFrame), y: valueAt(point.y, keyFrame) });
  let made: FusionPoint = { x: 0, y: 0 };
  if (points.length === 1) made = { x: tidy(now(points[0]).x + 0.1), y: tidy(now(points[0]).y) };
  else if (before !== undefined && next !== undefined) made = { x: tidy((now(before).x + now(next).x) / 2), y: tidy((now(before).y + now(next).y) / 2) };
  return [{ op: 'set_fusion_node', ...at, id: node.id, params: { points: [...points.slice(0, index), made, ...points.slice(index)] } }];
}

/**
 * Take the point `index` (counted from 0) of a Polygon away: the list of points set whole without
 * it, the others the objects that they are, their keys with them. Nothing where the polygon has no
 * such point, and for a node that is no Polygon.
 */
export function removePoint(at: FusionAt, node: FusionNode, index: number): Op[] {
  if (node.kind !== 'polygon') return [];
  const points = node.params.points;
  if (!Number.isInteger(index) || index < 0 || index >= points.length) return [];
  return [{ op: 'set_fusion_node', ...at, id: node.id, params: { points: points.filter((_, place) => place !== index) } }];
}

/** Whether a composition is the first one, as a clip new to the page has it. */
export function isFirst(composition: Composition): boolean {
  const first = firstComposition();
  // What a node of the first composition says, whatever order its fields are written in and whether or not the defaults are written.
  const plain = (node: FusionNode) => [node.id, node.kind, node.label, node.pass_through ?? false, node.pos ?? null];
  const ends = (link: FusionLink) => [link.from, link.to, link.input];
  return JSON.stringify(composition.nodes.map(plain)) === JSON.stringify(first.nodes.map(plain))
    && JSON.stringify(sortFusionLinks(composition.links).map(ends)) === JSON.stringify(first.links.map(ends));
}

/** Reset Fusion Composition (p. 1272): the first composition in the place of the clip's; nothing where it has that one, or none. */
export function resetComposition(at: FusionAt, clip: Pick<Clip, 'fusion'>): Op[] {
  return clip.fusion === undefined || isFirst(clip.fusion) ? [] : [{ op: 'set_fusion', ...at, value: firstComposition() }];
}

/** Take the composition of a clip away, so that it shows its source as a clip that never had one. */
export function removeComposition(at: FusionAt, clip: Pick<Clip, 'fusion'>): Op[] {
  return clip.fusion === undefined ? [] : [{ op: 'set_fusion', ...at, value: null }];
}

/** What a key asks of the node editor. */
export type FusionCommand = { kind: 'delete'; key: 'Delete' | 'Backspace' } | { kind: 'passThrough' | 'rename' | 'cancel' | 'selectAll' };

/**
 * The keys of the node editor, as Fusion has them: Delete deletes the nodes chosen or the link
 * chosen (Backspace only a link, as in the Color page's editor), Ctrl+P passes the nodes chosen
 * through, F2 renames the node chosen, Ctrl+A chooses every node, and Escape takes a drag back.
 * None of them is a key of the transport, which takes no key with Ctrl. A letter is known by where
 * it lies on the keyboard.
 */
export function fusionKey(key: Key): FusionCommand | null {
  if (key.composing || key.meta || key.repeat || key.alt || key.shift) return null;
  const letter = /^Key([A-Z])$/.exec(key.code ?? '')?.[1]?.toLowerCase() ?? (key.key.length === 1 ? key.key.toLowerCase() : '');
  if (key.ctrl) {
    if (letter === 'p') return { kind: 'passThrough' };
    if (letter === 'a') return { kind: 'selectAll' };
    return null;
  }
  if (key.key === 'Delete' || key.key === 'Backspace') return { kind: 'delete', key: key.key };
  if (key.key === 'F2') return { kind: 'rename' };
  if (key.key === 'Escape') return { kind: 'cancel' };
  return null;
}
