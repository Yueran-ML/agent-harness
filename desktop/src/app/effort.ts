/**
 * How hard the model thinks, as the page holds it: the levels that the status tells of, which of
 * them are kept for the next start, and what the program answers when one is kept. Nothing here
 * draws.
 */
import type { SessionStatus } from '../protocol';

/** Every level, the least first. */
export const efforts = ['default', 'off', 'low', 'high', 'max', 'ultra'] as const;
export type Effort = typeof efforts[number];

const isEffort = (value: unknown): value is Effort => efforts.some(level => level === value);

/**
 * The level in use and those that the service offers, in their order, as the status tells them.
 * Null where it tells of none, as an older program does, or tells of them in a way that cannot be
 * read.
 */
export function effortOf(model: SessionStatus['model'] | undefined): { effort: Effort; efforts: Effort[] } | null {
  const offered = Array.isArray(model?.efforts) ? efforts.filter(level => model.efforts!.includes(level)) : [];
  const effort = offered.find(level => level === model?.effort);
  return effort === undefined ? null : { effort, efforts: offered };
}

/** Whether a level is kept for the next start. Ultra holds for one run. */
export function keptLevel(effort: Effort): boolean {
  return effort !== 'ultra';
}

/** What the program answered when a level was to be kept: that it was, or what it said instead. */
export type EffortStored = { kind: 'stored' } | { kind: 'refused'; said: string };

/** Read what the program answers when a level is kept. What it does not say is a refusal. */
export function effortStoredOf(answer: unknown): EffortStored {
  const given = (answer !== null && typeof answer === 'object' ? answer : {}) as { effort?: unknown; error?: unknown };
  const stored = given.effort !== null && typeof given.effort === 'object' ? given.effort as { name?: unknown; effort?: unknown } : null;
  if (stored !== null && typeof stored.name === 'string' && isEffort(stored.effort)) return { kind: 'stored' };
  return { kind: 'refused', said: typeof given.error === 'string' ? given.error : '' };
}
