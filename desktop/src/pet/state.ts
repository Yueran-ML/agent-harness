/**
 * What the pet shows of the task that is open: one of a few states, with a short label. The same
 * states are written for the pets of other programs, see docs/pets.md in the repository.
 */
import type { Workspace } from '../useWorkspace';

export type PetState = 'idle' | 'thinking' | 'review' | 'waiting' | 'failed' | 'done';

export interface PetShown {
  state: PetState;
  /** The tool that runs or that waits for a yes, or the code of a failure. Empty where there is none. */
  label: string;
}

/** How long after it ended a turn is still shown as done, or as failed. */
export const AFTER_MS = 15_000;

/** The reasons for which a turn ends that are shown as a failure. A turn that was stopped is not one. */
const FAILED = new Set(['error', 'blocked', 'max-tokens']);

/**
 * What the pet shows of a task at a moment.
 *
 * A question of the agent comes first: the pet waits with the user. While a turn runs, the pet
 * reads where a tool runs and thinks otherwise. A turn that has just ended is shown as done or as
 * failed for a while, and then the pet is at rest again.
 */
export function petStateOf(
  workspace: Pick<Workspace, 'status' | 'approval' | 'projection'>, now: number,
): PetShown {
  if (workspace.approval !== null) return { state: 'waiting', label: workspace.approval.tool };
  const { projection } = workspace;
  if (workspace.status?.status === 'running') {
    for (let at = projection.items.length - 1; at >= 0; at -= 1) {
      const item = projection.items[at];
      if (item.type === 'tool' && item.result === null && !item.interrupted) return { state: 'review', label: item.name };
    }
    return { state: 'thinking', label: '' };
  }
  const end = projection.turns.at(-1)?.end;
  if (end && now - end.time < AFTER_MS) {
    if (end.reason === 'completed') return { state: 'done', label: '' };
    if (FAILED.has(end.reason)) return { state: 'failed', label: end.code ?? '' };
  }
  return { state: 'idle', label: '' };
}
