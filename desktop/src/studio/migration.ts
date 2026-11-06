/**
 * What became of the old projects of the folder when the studio was opened: the shell has them
 * migrated into timelines beside the studio, and tells the page once, where there was something
 * to say. It is kept until the user puts it away, also while the studio is left and shown again.
 */
import type { EngineBridge } from '../bridge';
import type { EngineEvent, MigrationReport } from './engine';

export interface Migrated {
  session: number;
  /** The old projects that became timelines. */
  done: MigrationReport[];
  /** Those that could not be migrated, each with why. */
  failed: MigrationReport[];
  /** Why the old projects could not be looked at, where they could not. */
  error: string | null;
}

export class Migrations {
  private told: Migrated | null = null;
  private readonly listeners = new Set<() => void>();

  constructor(bridge: Pick<EngineBridge, 'onEvent'>) {
    bridge.onEvent(event => this.heard(event));
  }

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  readonly getSnapshot = (): Migrated | null => this.told;

  /** Put away what was said: it is not said again. */
  dismiss(): void {
    this.told = null;
    this.changed();
  }

  answered(session: number, timeline: string): void {
    if (this.told?.session !== session) return;
    this.told = {...this.told, done: this.told.done.map(report => report.timeline === timeline ? {...report, duck_prompted: true, duck_pending: false} : report)};
    this.changed();
  }

  private heard(event: EngineEvent): void {
    if (event.kind !== 'migrated') return;
    const reports = Array.isArray(event.migrations) ? event.migrations : [];
    const done = reports.filter(report => typeof report.error !== 'string');
    const failed = reports.filter(report => typeof report.error === 'string');
    const error = typeof event.error === 'string' && event.error ? event.error : null;
    if (done.length === 0 && failed.length === 0 && error === null) return;
    this.told = { session: event.session, done, failed, error };
    this.changed();
  }

  private changed(): void {
    for (const listener of [...this.listeners]) listener();
  }
}

const made = new WeakMap<object, Migrations>();

/** What the page was told of the old projects, for the engine of the studio that the bridge reaches. */
export function migrationsOf(bridge: Pick<EngineBridge, 'onEvent'>): Migrations {
  let migrations = made.get(bridge);
  if (!migrations) {
    migrations = new Migrations(bridge);
    made.set(bridge, migrations);
  }
  return migrations;
}

/** The name of the folder of an old project: the timeline that it became is named after it. */
export function folderName(source: string): string {
  return source.split(/[\\/]+/).filter(Boolean).at(-1) ?? source;
}
