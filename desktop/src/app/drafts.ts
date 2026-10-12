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
  private readonly kept = new Map<string, string>();

  /** How many tasks have a draft. */
  get size(): number {
    return this.kept.size;
  }

  of(place: Place): string {
    return this.kept.get(nameOf(place)) ?? '';
  }

  keep(place: Place, text: string): void {
    if (text === '') this.kept.delete(nameOf(place));
    else this.kept.set(nameOf(place), text);
  }

  /**
   * Say that a text was sent from a task. Its draft is forgotten if it is
   * that text still. What was written there meanwhile stays.
   */
  sent(place: Place, text: string): void {
    if (this.of(place) === text) this.keep(place, '');
  }
}

// Written as JSON, so that no two places have one name whatever stands in theirs.
function nameOf(place: Place): string {
  return JSON.stringify([place.project, place.task]);
}
