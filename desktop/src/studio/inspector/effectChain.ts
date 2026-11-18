/**
 * The effects of a clip of picture as the Effects tab of the Inspector of the Edit page shows them (DaVinci Resolve Reference
 * Manual 20, the Effects tab of the Inspector): a Blur, a Mosaic (the Mosaic Blur of Resolve FX) and a 3D Keyer, each written
 * into the composition of the clip as a node of it (PROTOCOL.md, "Compositions"), the 3D Keyer as a `delta_keyer`. So the
 * engine has one way of drawing them, the Fusion page shows each as a node that it edits, and the agent's tools reach them
 * as they reach every node.
 *
 * The effects listed are the nodes of those kinds on the chain of the picture that ends in the MediaOut (`outputChain`), in
 * the order that the picture goes through them. An effect added goes in right before the MediaOut, after whatever reaches it
 * now, with its effect mask free; a clip that has no composition is given the first one (`withFirst`) in the same
 * transaction, so that one undo takes both away. An effect taken away is joined across as the node editor's Delete joins a
 * node (`removeNodes`), and one turned off is passed through. Pure, so that the Inspector and the tests work from the same
 * rules.
 */
import type { Clip, Composition, FusionKind, FusionLink, FusionNode, Op } from '../engine';
import { newNode, removeNodes, roomFor, shownComposition, withFirst, type FusionAt } from '../fusion/edits';
import { pictureInputs } from '../fusion/graph';

/** The kinds of node that the Inspector of the Edit page adds and lists as the effects of a clip, in the order of its menu. */
export const CLIP_EFFECTS = ['blur', 'mosaic', 'delta_keyer'] as const;
export type ClipEffect = (typeof CLIP_EFFECTS)[number];
/** The effects of a clip of a title or a colour, which has no screen to key. */
const PLAIN_EFFECTS: readonly ClipEffect[] = ['blur', 'mosaic'];

/** Whether a kind of node is one that the Inspector of the Edit page lists as an effect. */
export const isClipEffect = (kind: FusionKind): kind is ClipEffect => (CLIP_EFFECTS as readonly FusionKind[]).includes(kind);

/**
 * The effects that a clip of picture is offered: a Blur and a Mosaic on every one, as the engine gives a composition to a clip
 * of media, a title, a colour and a still; the 3D Keyer on a clip of media alone, whose picture may have a screen to key.
 */
export function effectsFor(clip: Pick<Clip, 'source'>): readonly ClipEffect[] {
  return clip.source.kind === 'media' ? CLIP_EFFECTS : PLAIN_EFFECTS;
}

/** The MediaOut of a composition, of which it has exactly one. */
const outputOf = (composition: Composition): FusionNode | undefined => composition.nodes.find(node => node.kind === 'media_out');

/** The link into an input of a node, where it has one. */
const linkInto = (composition: Composition, id: string, input: FusionLink['input']): FusionLink | undefined =>
  composition.links.find(link => link.to === id && link.input === input);

/**
 * The chain of the picture that ends in the MediaOut: the node that feeds the MediaOut, the node that feeds the first input of
 * the picture of that one (the `input` of an effect, the `background` of a Merge), and so on back to a node that has no such
 * input or none linked into it, which is the MediaIn of a plain composition. In the order that the picture goes through them,
 * from that first node; the MediaOut is not among them. A link that loops (which the engine refuses) ends the chain.
 */
export function outputChain(composition: Composition): FusionNode[] {
  const output = outputOf(composition);
  if (!output) return [];
  const chain: FusionNode[] = [];
  const seen = new Set<string>([output.id]);
  let link = linkInto(composition, output.id, 'input');
  while (link !== undefined && !seen.has(link.from)) {
    const from = link.from;
    const node = composition.nodes.find(each => each.id === from);
    if (!node) break;
    seen.add(node.id);
    chain.push(node);
    const first = pictureInputs(node.kind)[0];
    link = first === undefined ? undefined : linkInto(composition, node.id, first);
  }
  return chain.reverse();
}

/** The effects of a clip: the nodes of the chain of its picture that are a Blur, a Mosaic or a Delta Keyer, from the MediaIn side. */
export function chainEffects(composition: Composition): FusionNode[] {
  return outputChain(composition).filter(node => isClipEffect(node.kind));
}

/**
 * Whether a composition has nodes that the list of the effects does not show: any but its MediaOut, the MediaIn that the
 * chain of its picture begins at, and the effects on that chain. A Merge, a Transform, a mask, a Tracker, an effect off the
 * chain, or a second MediaIn, is one; so is the MediaIn of a composition whose MediaOut nothing feeds.
 */
export function hasMoreNodes(composition: Composition): boolean {
  const chain = outputChain(composition);
  const head = chain[0];
  const shown = new Set(chain.filter(node => isClipEffect(node.kind)).map(node => node.id));
  return composition.nodes.some(node => node.kind !== 'media_out' && !shown.has(node.id) && !(node.kind === 'media_in' && node.id === head?.id));
}

/**
 * Add an effect to a clip, right before the MediaOut of its composition: what feeds the MediaOut now feeds the first input of
 * the new node, and the new node feeds the MediaOut, in the place of the link that was there; nothing else that the node before
 * it fed is moved, and the effect mask of the new node is left free. Its settings are those that the engine gives a new one.
 * A clip that has no composition is shown with the first one, and is given it before the node, in the same transaction.
 * Nothing where the clip is not offered the kind (`effectsFor`), or its composition has no room for one node more.
 */
export function addEffect(at: FusionAt, clip: Pick<Clip, 'fusion' | 'source'>, kind: ClipEffect): Op[] {
  if (!effectsFor(clip).includes(kind)) return [];
  const composition = shownComposition(clip);
  const output = outputOf(composition);
  const first = pictureInputs(kind)[0];
  if (!output || first === undefined || !roomFor(composition, 1)) return [];
  const fed = linkInto(composition, output.id, 'input');
  const links: FusionLink[] = [
    ...(fed ? [{ from: fed.from, to: '', input: first }] : []),
    { from: '', to: output.id, input: 'input' },
  ];
  return withFirst(at, clip.fusion !== undefined, [{ op: 'add_fusion_node', ...at, node: newNode(kind), links }]);
}

/** Take an effect away, joining what fed it to what it fed, as Delete in the node editor does. Nothing where the clip has no composition. */
export function removeEffect(at: FusionAt, clip: Pick<Clip, 'fusion'>, id: string): Op[] {
  return clip.fusion === undefined ? [] : removeNodes(at, clip.fusion, [id]);
}

/** Turn an effect on or off: off, it is passed through, and hands on the picture that comes into it. Nothing where it is that already. */
export function enableEffect(at: FusionAt, clip: Pick<Clip, 'fusion'>, id: string, on: boolean): Op[] {
  const node = clip.fusion?.nodes.find(each => each.id === id);
  if (!node || (node.pass_through ?? false) === !on) return [];
  return [{ op: 'set_fusion_node', ...at, id, pass_through: !on }];
}
