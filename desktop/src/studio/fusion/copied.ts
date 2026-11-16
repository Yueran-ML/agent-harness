/**
 * The nodes that were copied on the Fusion page (Ctrl+C, Ctrl+X), kept for Paste. They are kept in
 * the page's own memory and not on the clipboard of the system, as the Color page keeps a node that
 * was copied; but here they are kept for the window, not for the page as it is shown: the node
 * editor is made anew for every clip, and the page for every timeline, and what was copied in the
 * composition of one clip is pasted into that of another.
 */
import type { CopiedNodes } from './edits';

let kept: CopiedNodes | null = null;

/** The nodes copied last, or null where none were. */
export function copiedNodes(): CopiedNodes | null {
  return kept;
}

/** Keep nodes for Paste, in the place of those kept before; null keeps none. */
export function keepCopied(copied: CopiedNodes | null): void {
  kept = copied;
}
