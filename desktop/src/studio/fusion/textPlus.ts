/**
 * A Text+ of a composition on the Fusion page (Fusion's Text+ generator; the settings of its Text,
 * Layout, Transform and Shading tabs and its Follower): the operations that set what takes no keys,
 * and that turn its parts on and off. Its numbers take keys, and are set as every setting of a node
 * that takes keys is (`settings.ts`).
 *
 * Its outline, its border and its Follower are parts that it has or has not: one is turned on by a
 * patch that gives it whole, as the engine makes a new one (`newOutline`, `newBorder`,
 * `newFollower`), and turned off by a patch that gives it `null`, which takes it away, its keys
 * with it. Each is one step to undo. A setting of a part that the Text+ has not is not there: it is
 * neither shown nor set. Pure.
 */
import { mergePatch, TEXT_PARTS, type FusionNode, type FusionTextPlus, type Op, type TextPart } from '../engine';
import { newBorder, newFollower, newOutline, type FusionAt } from './edits';

/** The most characters that the text of a Text+ may have (Unicode scalars, PROTOCOL.md). */
export const MOST_TEXT = 4096;
/** The range of the Delay of a Follower, in frames. */
export const FOLLOWER_DELAY = { neutral: 2, low: 0, high: 10000 } as const;

/** Text as a Text+ takes it: its first `MOST_TEXT` characters, each a Unicode scalar, a pair of surrogates one of them. */
export const limitedText = (text: string): string => {
  const characters = [...text];
  return characters.length <= MOST_TEXT ? text : characters.slice(0, MOST_TEXT).join('');
};

/** The settings of a node that is a Text+; null of a node of another kind. */
const textOf = (node: FusionNode): FusionTextPlus | null => (node.kind === 'text_plus' ? node.params : null);

/** Whether a Text+ has a part. */
export const hasPart = (params: FusionTextPlus, part: TextPart): boolean => typeof params[part] === 'object' && params[part] !== null;

/** A part of a Text+ as the engine makes one. */
const newPart = (part: TextPart) => (part === 'outline' ? newOutline() : part === 'border' ? newBorder() : newFollower());

/**
 * Settings of a Text+ that take no keys (its text, its font, how it is justified, what its Transform moves, the order and the
 * Delay of its Follower), set by one patch: nothing where the patch leaves the Text+ as it is, and nothing where it names a part
 * that the Text+ has not, which the patch would make half a part of. A part is turned on and off by `setPart`.
 */
export function setText(at: FusionAt, node: FusionNode, patch: Record<string, unknown>): Op[] {
  const params = textOf(node);
  if (params === null) return [];
  if (TEXT_PARTS.some(part => patch[part] !== undefined && !hasPart(params, part))) return [];
  const after = mergePatch(params, patch);
  return JSON.stringify(after) === JSON.stringify(params) ? [] : [{ op: 'set_fusion_node', ...at, id: node.id, params: patch }];
}

/**
 * Turn a part of a Text+ on, as the engine makes a new one, or off, which takes it away with its keys: one patch each. Nothing
 * where it is so already, and for a node of another kind.
 */
export function setPart(at: FusionAt, node: FusionNode, part: TextPart, on: boolean): Op[] {
  const params = textOf(node);
  if (params === null || hasPart(params, part) === on) return [];
  return [{ op: 'set_fusion_node', ...at, id: node.id, params: { [part]: on ? newPart(part) : null } }];
}
