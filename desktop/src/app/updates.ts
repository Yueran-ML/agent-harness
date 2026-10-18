/**
 * Newer versions of the application, as the page holds them: what the shell says of them, read
 * with care, and the two choices of the user that go with them, which the page keeps. Nothing
 * here draws.
 */

/** Why it is not known whether there is a newer version: GitHub was not reached, turned the question away, or gave no list. */
export const updateFailures = ['network', 'busy', 'unreadable'] as const;
export type UpdateFailure = typeof updateFailures[number];

/** The version of the application that runs, and whether it looks for newer ones. */
export interface About {
  version: string;
  checks: boolean;
}

/** A newer version, and the page of its release. */
export interface Newer {
  version: string;
  page: string;
}

/** What a question for a newer version found: one, or none, or why that is not known. */
export interface Found {
  newer: Newer | null;
  failure: UpdateFailure | null;
}

/** When the first question is asked once the window is open, and how often after that, in milliseconds. */
export const UPDATE_FIRST_MS = 5_000;
export const UPDATE_EVERY_MS = 24 * 60 * 60_000;

/** A version as a release of the application names it. */
const VERSION = /^\d{1,6}\.\d{1,6}\.\d{1,6}$/;

const fields = (answer: unknown) => (answer !== null && typeof answer === 'object' ? answer : {}) as Record<string, unknown>;

/** Read what the shell says of the application. Null for no such answer. */
export function aboutOf(answer: unknown): About | null {
  const given = fields(answer);
  if (typeof given.version !== 'string' || !/^\S{1,40}$/.test(given.version) || typeof given.checks !== 'boolean') return null;
  return { version: given.version, checks: given.checks };
}

/** Read what a question for a newer version found. What cannot be read is a list that did not come. */
export function foundOf(answer: unknown): Found {
  const given = fields(answer);
  const unread: Found = { newer: null, failure: 'unreadable' };
  if (given.newer === null) {
    const failure = given.failure === null ? null : updateFailures.find(kind => kind === given.failure);
    return failure === undefined ? unread : { newer: null, failure };
  }
  // The page of a release is one that the shell names, over https.
  if (typeof given.newer !== 'string' || !VERSION.test(given.newer) || typeof given.page !== 'string'
    || !/^https:\/\/\S+$/.test(given.page) || given.failure !== null) return unread;
  return { newer: { version: given.newer, page: given.page }, failure: null };
}

/** The newer version that the bar of the window points to: one that was found, and not skipped. */
export function toldOf(found: Found | null, skipped: string | null): Newer | null {
  const newer = found?.newer ?? null;
  return newer !== null && newer.version !== skipped ? newer : null;
}

const checkingKey = 'columbina-check-updates';
const skippedKey = 'columbina-update-skipped';

/** Whether the application looks for a newer version by itself, which it does unless the user said no. */
export function storedChecking(): boolean {
  try {
    return localStorage.getItem(checkingKey) !== 'false';
  } catch {
    return true;
  }
}

export function storeChecking(checking: boolean): void {
  try {
    localStorage.setItem(checkingKey, String(checking));
  } catch {
    // Without storage the choice holds for this window.
  }
}

/** The version that the user said not to be told of, if any. */
export function storedSkipped(): string | null {
  try {
    const kept = localStorage.getItem(skippedKey);
    return kept !== null && VERSION.test(kept) ? kept : null;
  } catch {
    return null;
  }
}

export function storeSkipped(version: string): void {
  try {
    localStorage.setItem(skippedKey, version);
  } catch {
    // Without storage the choice holds for this window.
  }
}
