/** What is written in the field and not sent. Every task keeps its own. */

/**
 * Where a draft belongs: a task of a project. The service knows a task by
 * both, and two projects may each have a task of the same name.
 */
export interface Place {
  /** The folder of the project, or null while it is not known. */
  project: string | null;
  task: string;
}

export function samePlace(one: Place | null, other: Place | null): boolean {
  if (one === null || other === null) return one === other;
  return one.project === other.project && one.task === other.task;
}

export class Drafts {
  private readonly kept = new Map<string, { text: string; returned: boolean }>();

  /** How many tasks have a draft. */
  get size(): number {
    return this.kept.size;
  }

  of(place: Place): string {
    return this.kept.get(nameOf(place))?.text ?? '';
  }

  /** Whether the draft of a task holds what was sent from it and never read. */
  returned(place: Place): boolean {
    return this.kept.get(nameOf(place))?.returned ?? false;
  }

  /**
   * Keep what is written for a task.
   *
   * Args:
   *   place: The task.
   *   text: What is written. Nothing written is no draft.
   *   returned: Whether it holds what was sent from the task and never read.
   */
  keep(place: Place, text: string, returned = false): void {
    if (text === '') this.kept.delete(nameOf(place));
    else this.kept.set(nameOf(place), { text, returned });
  }

  /**
   * Say that a text was sent from a task. Its draft is forgotten if it is
   * that text still. What was written there meanwhile stays.
   */
  sent(place: Place, text: string): void {
    if (this.of(place) === text) this.keep(place, '');
  }
}

/**
 * What waits unread in the task that is shown, for as long as its turn is under way.
 *
 * A task that is left with a turn under way has that turn stopped by the service, which
 * drops what waits. Nobody is told: the conversation has gone on to another task by then,
 * or the service process is ending. So the window remembers what it saw waiting, and what a
 * task was left with becomes a part of its draft.
 *
 * A task at rest is another matter. What waits in it is not dropped when it is left. It
 * waits in the log, and is there again when the task is opened.
 */
export class Unread {
  private place: Place | null = null;
  private waiting: readonly string[] = [];

  /**
   * Note what waits in the task that is shown. This is called with every look at the page.
   * Once another task is shown, what was seen of the one before stays until it is taken.
   */
  see(place: Place | null, running: boolean, waiting: readonly string[]): void {
    if (samePlace(place, this.place)) this.waiting = place !== null && running ? waiting : [];
  }

  /** Take what the task before was left with, and go on with the task that is shown now. */
  take(place: Place | null, running: boolean, waiting: readonly string[]): readonly string[] {
    const left = this.waiting;
    this.place = place;
    this.waiting = place !== null && running ? waiting : [];
    return left;
  }
}

// Written as JSON, so that no two places have one name whatever stands in theirs.
export function nameOf(place: Place): string {
  return JSON.stringify([place.project, place.task]);
}
