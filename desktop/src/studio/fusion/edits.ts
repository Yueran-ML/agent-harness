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
  firstComposition, sortFusionLinks, type Clip, type Composition, type FusionBackground, type FusionBlur, type FusionDeltaKeyer, type FusionEllipse, type FusionFollow,
  type FusionKind, type FusionLink, type FusionMaskCommon, type FusionMerge, type FusionMosaic, type FusionNode, type FusionPoint, type FusionPolygon,
  type FusionRectangle, type FusionTracker, type FusionTrackerPattern, type FusionTransform, type NodePos, type Op,
} from '../engine';
import { valueAt } from '../keyframes/value';
import {
  handedOn, hasMaskInput, hasOutput, isMaskKind, isMatteInput, isPictureInput, layoutComposition, MASK_KINDS, pictureInputs, placesOf, type MaskKind,
} from './graph';

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
/**
 * The effects on a picture that the editor adds, in a group of their own: a Blur, a Mosaic, a Delta Keyer and a Tracker. Each has
 * a picture, `input`, so it goes in after the node chosen as a Transform does (`addNode`).
 */
export const ADDABLE_EFFECTS: readonly Extract<FusionKind, 'blur' | 'mosaic' | 'delta_keyer' | 'tracker'>[] = ['blur', 'mosaic', 'delta_keyer', 'tracker'];
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
/**
 * The settings of a Delta Keyer as the engine makes a new one (PROTOCOL.md, "The Delta Keyer"): a pure green screen keyed at a
 * Gain of 1, the thresholds at their ends and the matte as the key made it, what the key took filled softly with a mid grey,
 * the spill suppressed whole by the Medium method, and the keyed picture shown.
 */
export const newDeltaKeyer = (): FusionDeltaKeyer => ({
  screen: { r: 0, g: 1, b: 0 }, gain: 1, balance: 0.5, pre_blur: 0, low: 0, high: 1, erode_dilate: 0, matte_blur: 0, clean_foreground: 0, clean_background: 0,
  replace: 'soft_color', replace_color: { r: 0.5, g: 0.5, b: 0.5 }, garbage_invert: false, spill_method: 'medium', spill_suppression: 1, view: 'final_result',
});

/** The most patterns that a Tracker may have (PROTOCOL.md, "The Tracker"). */
export const MOST_TRACKERS = 16;
/**
 * A pattern of a Tracker as the engine makes one (`Pattern::new` of `crates/model/src/fusion/tracker.rs`), put at `x` and `y`:
 * on, with no name, a twentieth of the picture wide and high, searched for in an eighth of it, with no offset and no path.
 */
export const newPattern = (x = 0.5, y = 0.5): FusionTrackerPattern => ({
  name: '', enabled: true, pattern: { x, y, width: 0.05, height: 0.05 }, search: { width: 0.125, height: 0.125 }, offset: { x: 0, y: 0 }, path: {},
});
/**
 * The settings of a Tracker as the engine makes a new one (PROTOCOL.md, "The Tracker"): an Operation of None, BG Only, the three
 * boxes of the motion on, the pivot at the tracker average, frame 0 of the keys as the reference, and no patterns.
 */
export const newTracker = (): FusionTracker => ({
  operation: 'none', merge: 'bg_only', position: true, rotation: true, scaling: true, pivot: 'tracker_average', pivot_tracker: 0,
  pivot_point: { x: 0.5, y: 0.5 }, reference: 0, trackers: [],
});

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
    case 'delta_keyer': return { ...base, kind, params: newDeltaKeyer() };
    case 'tracker': return { ...base, kind, params: newTracker() };
    default: return { ...base, kind, params: {} };
  }
}

/** The kinds of node that have a centre, which may follow a Tracker (PROTOCOL.md, "Following a Tracker"). */
export const CENTER_KINDS = ['merge', 'transform', 'rectangle', 'ellipse', 'polygon'] as const;
export type CenterKind = (typeof CENTER_KINDS)[number];
/** Whether a kind of node has a centre. */
export const hasCenter = (kind: FusionKind): kind is CenterKind => (CENTER_KINDS as readonly FusionKind[]).includes(kind);

/** What the centre of a node follows, or null: a node of a kind that has no centre follows nothing. */
export function followOf(node: FusionNode): FusionFollow | null {
  switch (node.kind) {
    case 'merge': case 'transform': case 'rectangle': case 'ellipse': case 'polygon': return node.params.center?.follow ?? null;
    default: return null;
  }
}

/** A node with its centre following `follow`, or following nothing where it is null: of a kind that has no centre, the node as it is. */
export function withFollow(node: FusionNode, follow: FusionFollow | null): FusionNode {
  if (!hasCenter(node.kind)) return node;
  const params = node.params as { center: { x: unknown; y: unknown; follow?: FusionFollow } };
  const { follow: _was, ...center } = params.center;
  return { ...node, params: { ...params, center: follow === null ? center : { ...center, follow } } } as FusionNode;
}

/** Whether two follows are the same: of the same node, the same output, and of the output `tracker` the same pattern. */
const sameFollow = (a: FusionFollow, b: FusionFollow): boolean => a.node === b.node && a.output === b.output && a.tracker === b.tracker;

/**
 * Have the centre of a node follow a Tracker (Connect To), or follow nothing (`null`, Disconnect): `set_fusion_node` with
 * `{"center": {"follow": ...}}`, a patch that leaves the centre's own `x` and `y` as they are, the pattern always written.
 * Nothing where it follows that already, or a node of a kind that has no centre.
 */
export function followOps(at: FusionAt, node: FusionNode, follow: FusionFollow | null): Op[] {
  if (!hasCenter(node.kind)) return [];
  const now = followOf(node);
  if (follow === null) return now === null ? [] : [{ op: 'set_fusion_node', ...at, id: node.id, params: { center: { follow: null } } }];
  if (now !== null && sameFollow(now, follow)) return [];
  return [{ op: 'set_fusion_node', ...at, id: node.id, params: { center: { follow: { node: follow.node, output: follow.output, tracker: follow.tracker } } } }];
}

/** The nodes whose centres follow one of the nodes `ids`, in the order of the composition. */
export const followersOf = (composition: Composition, ids: readonly string[]): FusionNode[] => composition.nodes.filter((node) => {
  const follow = followOf(node);
  return follow !== null && ids.includes(follow.node);
});

/**
 * Add a node. With `after`, a node that has an output, a node that has an input of a picture is
 * inserted after it: the links of the operation put the output of `after` into the first input of
 * the new node (`input` of a Transform, a Blur, a Mosaic, a Delta Keyer and a Tracker, `background` of a Merge), and the
 * output of the new node into every input that `after` fed, each in the place of the link that was there. A node without
 * such an input (a Background, a MediaIn), or one added with no node chosen, is added with no links. A Tracker begins
 * with one pattern in the middle of the picture (`newPattern`), as a Polygon begins with a square: the engine makes one
 * with none, and a Tracker without a pattern has nothing to place on the picture.
 *
 * A mask (a Rectangle, an Ellipse, a Polygon) is not put into the chain of the picture: as in
 * Fusion, where the node chosen has a `mask` input that nothing is linked into, the mask is linked
 * into it, in the same operation, and else it is added with no links. A Polygon begins with the
 * points of `startPoints`. A mask added with a Delta Keyer chosen goes into its effect mask as into
 * that of any node, never into one of its mattes: which of the two a mask is to be is not the
 * editor's to say, so a matte is linked by hand.
 *
 * The new node is named by the empty id in its links, as `add_fusion_node` takes them.
 */
export function addNode(at: FusionAt, composition: Composition, kind: FusionKind, after: string | null, pos: NodePos | null): Op[] {
  const base = { id: '', label: '', pass_through: false, pos: pos ? placeAt(pos) : null };
  const node: FusionNode = kind === 'polygon' ? { ...base, kind, params: { ...newPolygon(), points: startPoints() } }
    : kind === 'tracker' ? { ...base, kind, params: { ...newTracker(), trackers: [newPattern()] } }
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

/** The ids of the nodes that a transaction added, in the order they were added: those pasted, which become the ones chosen. */
export function addedNodes(done: readonly Op[] | null | undefined): string[] {
  return (done ?? []).filter((op): op is Extract<Op, { op: 'add_fusion_node' }> => op.op === 'add_fusion_node').map(op => op.node.id);
}

/** The id of the node that a transaction added, which becomes the one chosen. */
export function addedNode(done: readonly Op[] | null | undefined): string | null {
  return addedNodes(done).at(-1) ?? null;
}

/** The most nodes that a composition may have (PROTOCOL.md, "Compositions"). */
export const MOST_NODES = 256;
/** Whether a composition has room for `count` nodes more. */
export const roomFor = (composition: Composition, count: number): boolean => composition.nodes.length + count <= MOST_NODES;

/** How far down and right of where they were nodes are pasted, so that a copy does not hide what it was copied from. */
export const PASTE_OFFSET: NodePos = { x: 24, y: 24 };

/**
 * Nodes that were copied (Ctrl+C), as they are kept for Paste: each whole, with its settings, its
 * keys, its label and whether it is passed through, at the place where the editor drew it (its
 * `pos`, or where it was laid out), and the links among them. They are named by the ids that they
 * had where they were copied, which tell only which link joins which two of them.
 */
export interface CopiedNodes { nodes: FusionNode[]; links: FusionLink[] }

/**
 * Copy nodes: those of `ids` that the composition has, in the order of the composition, but never
 * the MediaOut, of which a composition has exactly one; and the links that both begin and end
 * among them. Null where there is nothing to copy.
 */
export function copyNodes(composition: Composition, ids: readonly string[]): CopiedNodes | null {
  const nodes = layoutComposition(composition, placesOf(composition)).nodes
    .filter(each => ids.includes(each.node.id) && each.node.kind !== 'media_out')
    .map((each): FusionNode => ({ ...structuredClone(each.node), pos: placeAt({ x: each.x, y: each.y }) }));
  if (nodes.length === 0) return null;
  const taken = new Set(nodes.map(node => node.id));
  const links = sortFusionLinks(composition.links).filter(link => taken.has(link.from) && taken.has(link.to)).map(link => ({ ...link }));
  return { nodes, links };
}

/** The name of a transaction (`$name`, PROTOCOL.md) that the node pasted at a place of the copy is given, for the links of the nodes after it. */
export const pastedName = (index: number): string => `$p${index}`;

/** How far nodes that lay at `places` are moved where they are pasted: to the point `to`, or down and right, clear of the nodes that are there (see `pasteNodes`). */
function pasteOffset(composition: Composition, places: readonly NodePos[], to: NodePos | null): NodePos {
  if (to !== null) return { x: to.x - Math.min(...places.map(place => place.x)), y: to.y - Math.min(...places.map(place => place.y)) };
  const spot = (x: number, y: number) => `${Math.round(x)},${Math.round(y)}`;
  const there = new Set(layoutComposition(composition, placesOf(composition)).nodes.map(each => spot(each.x, each.y)));
  // Each step tries places that no step before tried, and the nodes that are there are few: one of them is clear.
  for (let step = 1; ; step++) {
    const by = { x: PASTE_OFFSET.x * step, y: PASTE_OFFSET.y * step };
    if (!places.some(place => there.has(spot(place.x + by.x, place.y + by.y)))) return by;
  }
}

/**
 * Paste nodes that were copied: one `add_fusion_node` for each, in the order they were copied in,
 * each with the settings, keys, label and Pass Through that it had, and with the links among them;
 * nothing is linked to the rest of the composition. The engine gives each a new id. So that the
 * links can name nodes that have no id yet, each node is given a name of the transaction
 * (`$p0`, `$p1`, …), and a link is sent with the later of the two nodes that it joins, as
 * `add_fusion_node` takes links: each touches the node added, which it names by the name given,
 * and its other end names a node added before.
 *
 * With `to`, a point of the graph (Paste from the menu, where it was opened), the nodes lie there:
 * the corner of the box about them, the others as they lay about it. Without, they lie down and
 * right of where they were (`PASTE_OFFSET`), and further again while one would lie exactly on a
 * node that is there, so that the same nodes pasted twice do not hide each other.
 *
 * The keys are kept as they are: they count the frames of the clip that they were copied from.
 * Nothing where nothing was copied, and where the composition has no room (`roomFor`).
 *
 * A centre that follows a Tracker (PROTOCOL.md, "Following a Tracker") follows the copy of that
 * Tracker where it is pasted with it, named by its name of the transaction, which the engine takes
 * for the `node` that a centre follows: in the node as it is added where the Tracker was added
 * before it, and else by a `set_fusion_node` of the centre after the last node is added. Where the
 * Tracker is not pasted with it, the centre follows it still where the composition has it (a node
 * of that id that is a Tracker), and follows nothing else, which the engine would refuse as
 * `missing`.
 */
export function pasteNodes(at: FusionAt, composition: Composition, copied: CopiedNodes | null, to: NodePos | null = null): Op[] {
  if (copied === null || copied.nodes.length === 0 || !roomFor(composition, copied.nodes.length)) return [];
  const places: NodePos[] = copied.nodes.map(node => node.pos ?? { x: 0, y: 0 });
  const by = pasteOffset(composition, places, to);
  const placeOf = new Map<string, number>(copied.nodes.map((node, index) => [node.id, index]));
  // The centres that follow a Tracker that is added after them, set once it is there.
  const later: Op[] = [];
  const added = copied.nodes.map((node, index): Op => {
    const links = copied.links.flatMap((link): FusionLink[] => {
      const from = placeOf.get(link.from);
      const into = placeOf.get(link.to);
      return from !== undefined && into !== undefined && from !== into && Math.max(from, into) === index
        ? [{ from: pastedName(from), to: pastedName(into), input: link.input }] : [];
    });
    const pos = placeAt({ x: Math.max(0, places[index].x + by.x), y: Math.max(0, places[index].y + by.y) });
    let made: FusionNode = { ...structuredClone(node), id: pastedName(index), pos };
    const follow = followOf(made);
    if (follow !== null) {
      const tracker = placeOf.get(follow.node);
      if (tracker !== undefined && copied.nodes[tracker].kind === 'tracker') {
        const named: FusionFollow = { ...follow, node: pastedName(tracker) };
        if (tracker < index) made = withFollow(made, named);
        else {
          made = withFollow(made, null);
          later.push({ op: 'set_fusion_node', ...at, id: pastedName(index), params: { center: { follow: named } } });
        }
      } else if (tracker !== undefined || !composition.nodes.some(each => each.id === follow.node && each.kind === 'tracker')) {
        made = withFollow(made, null);
      }
    }
    return links.length > 0 ? { op: 'add_fusion_node', ...at, node: made, links } : { op: 'add_fusion_node', ...at, node: made };
  });
  return [...added, ...later];
}

/**
 * Cut nodes (Ctrl+X): what Copy keeps of them, and the operations that take them away as Delete
 * does (`removeNodes`), joining what fed each to what it fed where that is a single chain. The
 * MediaOut is neither copied nor taken away.
 */
export function cutNodes(at: FusionAt, composition: Composition, ids: readonly string[]): { copied: CopiedNodes | null; ops: Op[] } {
  const copied = copyNodes(composition, ids);
  return { copied, ops: copied === null ? [] : removeNodes(at, composition, copied.nodes.map(node => node.id)) };
}

/** Duplicate nodes (Ctrl+D): Copy and Paste in one, which leaves what was copied before as it is. */
export function duplicateNodes(at: FusionAt, composition: Composition, ids: readonly string[]): Op[] {
  return pasteNodes(at, composition, copyNodes(composition, ids));
}

/**
 * Put a node into a link, as Fusion does where a node is dragged onto one: the link `A → B.input`
 * becomes `A → node.first` and `node → B.input`, the second in the place of the link that was
 * there. Only a node that has an input of a picture and an output and no link at all goes in so
 * (a mask, a Background and a MediaIn have no such input, the MediaOut no output), and only into a
 * link of a picture: a link into a `mask`, or into a matte of a Delta Keyer, is left as it is.
 * Nothing otherwise.
 */
export function insertOnLink(at: FusionAt, composition: Composition, id: string, link: FusionLink): Op[] {
  const node = composition.nodes.find(each => each.id === id);
  if (!node || !hasOutput(node.kind) || !isPictureInput(link.input) || link.from === id || link.to === id) return [];
  const first = pictureInputs(node.kind)[0];
  if (first === undefined || composition.links.some(each => each.from === id || each.to === id)) return [];
  if (!composition.links.some(each => each.from === link.from && each.to === link.to && each.input === link.input)) return [];
  return [
    { op: 'connect_fusion', ...at, link: { from: link.from, to: id, input: first } },
    { op: 'connect_fusion', ...at, link: { from: id, to: link.to, input: link.input } },
  ];
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
 * and is counted. The mattes of a Delta Keyer are not counted either: it is joined across by its
 * `input` alone, and what fed its mattes is joined to nothing; nor is the `foreground` of a Tracker,
 * which it makes follow its patterns and does not hand on. Each node is taken away from the
 * composition as the ones before it left it, as the engine applies them.
 *
 * A Tracker that the centre of a node follows is not taken away by the engine (`in_use`, PROTOCOL.md,
 * "Following a Tracker"): every centre that follows one of the Trackers deleted lets go of it first,
 * in the same transaction (`{"center": {"follow": null}}`), so that the whole is one step to undo and
 * the undo puts each centre back on the Tracker that it followed.
 */
export function removeNodes(at: FusionAt, composition: Composition, ids: readonly string[]): Op[] {
  let now = composition;
  const ops: Op[] = [];
  for (const id of ids) {
    const node = now.nodes.find(each => each.id === id);
    if (!node || node.kind === 'media_out') continue;
    const passed = handedOn(node.kind);
    const into = now.links.filter(link => link.to === id && !isMatteInput(link.input) && (link.input !== 'mask' || passed === 'mask')
      && !(node.kind === 'tracker' && link.input === 'foreground'));
    const fed = now.links.filter(link => link.from === id);
    const links: FusionLink[] = into.length === 1 ? fed.map(link => ({ from: into[0].from, to: link.to, input: link.input })) : [];
    ops.push(links.length > 0 ? { op: 'remove_fusion_node', ...at, id, links } : { op: 'remove_fusion_node', ...at, id });
    now = { nodes: now.nodes.filter(each => each.id !== id), links: sortFusionLinks([...now.links.filter(link => link.from !== id && link.to !== id), ...links]) };
  }
  const trackers = composition.nodes.filter(node => node.kind === 'tracker' && ids.includes(node.id)).map(node => node.id);
  const letGo = followersOf(composition, trackers).flatMap(node => followOps(at, node, null));
  return ops.length === 0 ? [] : [...letGo, ...ops];
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

/**
 * The node that the viewer shows once the node `id` is asked to be viewed (the key `1`, and View
 * in the menu of a node), where it shows `viewed` now; null is what comes out of the composition.
 * A node that is viewed already goes back to the output, as the key pressed again does in Fusion;
 * the MediaOut is the output itself; and a node that the composition has not is none to view.
 */
export function viewedAfter(composition: Composition, viewed: string | null, id: string): string | null {
  const node = composition.nodes.find(each => each.id === id);
  return !node || node.kind === 'media_out' || viewed === id ? null : id;
}

/** What a key asks of the node editor. */
export type FusionCommand = { kind: 'delete'; key: 'Delete' | 'Backspace' }
  | { kind: 'passThrough' | 'rename' | 'cancel' | 'selectAll' | 'copy' | 'cut' | 'paste' | 'duplicate' | 'selectTool' | 'view' | 'viewOutput' };

/**
 * The keys of the node editor, as Fusion has them: Delete deletes the nodes chosen or the link
 * chosen (Backspace only a link, as in the Color page's editor), Ctrl+P passes the nodes chosen
 * through, F2 renames the node chosen, Ctrl+A chooses every node, and Escape takes a drag back.
 * Ctrl+C, Ctrl+X, Ctrl+V and Ctrl+D copy, cut, paste and duplicate nodes, with Cmd in the place of
 * Ctrl too, as a Mac has them. Shift+Space opens Select Tool: Space alone is the transport's, which
 * would take it with Shift as well, so the editor takes this one key before the transport does. A
 * letter is known by where it lies on the keyboard. The key `1` shows the node chosen in the viewer,
 * as it does in Fusion (the left viewer), and `0` what comes out of the composition again; a digit
 * is known by what it types, or by where it lies in the row of digits.
 */
export function fusionKey(key: Key): FusionCommand | null {
  if (key.composing || key.repeat || key.alt) return null;
  if (key.shift) return !key.ctrl && !key.meta && (key.code === 'Space' || key.key === ' ') ? { kind: 'selectTool' } : null;
  const letter = /^Key([A-Z])$/.exec(key.code ?? '')?.[1]?.toLowerCase() ?? (key.key.length === 1 ? key.key.toLowerCase() : '');
  if (key.ctrl !== key.meta) {
    if (letter === 'c') return { kind: 'copy' };
    if (letter === 'x') return { kind: 'cut' };
    if (letter === 'v') return { kind: 'paste' };
    if (letter === 'd') return { kind: 'duplicate' };
    if (key.meta) return null;
    if (letter === 'p') return { kind: 'passThrough' };
    if (letter === 'a') return { kind: 'selectAll' };
    return null;
  }
  if (key.ctrl) return null;
  if (key.key === 'Delete' || key.key === 'Backspace') return { kind: 'delete', key: key.key };
  if (key.key === 'F2') return { kind: 'rename' };
  if (key.key === 'Escape') return { kind: 'cancel' };
  if (key.key === '1' || key.code === 'Digit1') return { kind: 'view' };
  if (key.key === '0' || key.code === 'Digit0') return { kind: 'viewOutput' };
  return null;
}
