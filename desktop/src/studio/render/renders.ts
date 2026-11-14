/**
 * The export of the window, as the page follows it: started from the dialog (the shell asks where
 * in Save As), told of by the engine as it goes, cancelled, and its file shown in its folder. It is
 * kept for as long as the page lives, not for as long as the studio shows: an export goes on while
 * the user edits, or leaves the studio, and is found as it is when the studio is shown again.
 */
import type { EngineBridge } from '../../bridge';
import { callEngine, type EngineEvent, type RenderCodec, type RenderOptions, type RenderProgress } from '../engine';

/** Why the engine (or the shell) would not start an export, as the page says it. */
export type RenderRefusal = 'busy' | 'exists' | 'part_exists' | 'source' | 'invalid' | 'other';

/** What came of asking for an export. */
export type RenderStart =
  | { kind: 'started' }
  /** Save As was cancelled: nothing was asked of the engine. */
  | { kind: 'cancelled' }
  | { kind: 'refused'; reason: RenderRefusal; detail: string };

/** The export of the window, from the moment it was started. */
export interface RenderJob {
  job: string;
  /** The session of the engine that renders it. */
  session: number;
  /**
   * The timeline, by its name as it was when it was started, and the codec that was asked for:
   * none for the sound alone.
   */
  name: string;
  codec: RenderCodec | null;
  state: RenderProgress['state'] | 'cancelling' | 'stopped';
  frame: number;
  frames: number;
  fps: number;
  path: string | null;
  encoder: string | null;
  message: string | null;
}

/** How many tellings of exports that the page has not started yet are kept: they may come before the answer that names their job. */
const EARLY_MOST = 16;

export class Renders {
  private job: RenderJob | null = null;
  private session: number | null = null;
  private asking = false;
  /** What the engine told of jobs of this session that are not the page's (yet): an answer may come after them. */
  private early = new Map<string, RenderProgress>();
  private readonly listeners = new Set<() => void>();
  private snapshot: { job: RenderJob | null; asking: boolean } = { job: null, asking: false };

  constructor(private readonly bridge: Pick<EngineBridge, 'call' | 'onEvent' | 'render' | 'reveal'>) {
    bridge.onEvent(event => this.heard(event));
  }

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  readonly getSnapshot = () => this.snapshot;

  /** Whether an export of the window is going on: one more is not asked for until it ends. */
  running(): boolean {
    const state = this.job?.state;
    return this.asking || state === 'running' || state === 'cancelling';
  }

  /**
   * Ask for an export: the shell asks where in Save As, and the engine starts it. `name` is the name
   * of the timeline, which the file is named after and the progress shows; `session` that of the
   * engine that is open.
   */
  async start(options: RenderOptions & { name: string }, session: number): Promise<RenderStart> {
    if (this.running()) return { kind: 'refused', reason: 'busy', detail: '' };
    this.asking = true;
    this.changed();
    try {
      const answer = await this.bridge.render(options);
      if (!answer.ok) return refusalOf(answer.error.code, answer.error.data, answer.error.message);
      const result = answer.result;
      if ('cancelled' in result) return { kind: 'cancelled' };
      const told = this.early.get(result.job);
      this.early.clear();
      this.job = {
        job: result.job, session, name: options.name, codec: options.codec ?? null,
        state: 'running', frame: 0, frames: 0, fps: 0, path: null, encoder: null, message: null,
      };
      if (told) this.take(told);
      return { kind: 'started' };
    } catch (failure) {
      // What the shell refused itself, such as a path that is not whole.
      return { kind: 'refused', reason: 'other', detail: failure instanceof Error ? failure.message : String(failure) };
    } finally {
      this.asking = false;
      this.changed();
    }
  }

  /** Cancel the export: it ends as the engine tells, once its files are cleaned away. */
  async cancel(): Promise<void> {
    const job = this.job;
    if (!job || job.state !== 'running') return;
    this.job = { ...job, state: 'cancelling' };
    this.changed();
    try {
      await callEngine(this.bridge, 'render.cancel', { job: job.job });
    } catch {
      // It ended meanwhile, or its engine did: what it was told last stands.
      if (this.job?.job === job.job && this.job.state === 'cancelling') this.job = { ...this.job, state: 'running' };
      this.changed();
    }
  }

  /** Show the file that the export wrote in its folder. */
  reveal(): Promise<boolean> {
    return this.job ? this.bridge.reveal(this.job.job) : Promise.resolve(false);
  }

  /** Put away what is said of an export that has ended. */
  dismiss(): void {
    if (this.job && !this.running()) {
      this.job = null;
      this.changed();
    }
  }

  private heard(event: EngineEvent): void {
    if (event.kind === 'opened') {
      if (this.session !== event.session) this.early.clear();
      this.session = event.session;
      if (this.job && this.job.session !== event.session) this.stopped();
      return;
    }
    if (event.kind === 'exited') {
      if (this.job?.session === event.session) this.stopped();
      return;
    }
    if (event.kind !== 'notification' || event.method !== 'render.progress') return;
    const told = event.params as RenderProgress | null;
    if (told === null || typeof told !== 'object' || typeof told.job !== 'string') return;
    if (this.job && this.job.session === event.session && this.job.job === told.job) {
      this.take(told);
      this.changed();
    } else if (this.asking && (this.session === null || event.session === this.session)) {
      // The engine tells of a job before the answer that names it comes through the shell.
      this.early.set(told.job, told);
      if (this.early.size > EARLY_MOST) this.early.delete(this.early.keys().next().value!);
    }
  }

  /** Take what the engine told of the export. A job that has ended stays as it ended. */
  private take(told: RenderProgress): void {
    const job = this.job;
    if (!job || job.state === 'done' || job.state === 'failed' || job.state === 'cancelled' || job.state === 'stopped') return;
    const number = (value: unknown, kept: number) => (typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : kept);
    const state = told.state === 'running' && job.state === 'cancelling' ? 'cancelling' : told.state;
    this.job = {
      ...job,
      state: ['running', 'done', 'failed', 'cancelled', 'cancelling'].includes(state) ? state : job.state,
      frame: number(told.frame, job.frame),
      frames: number(told.frames, job.frames),
      fps: number(told.fps, job.fps),
      path: typeof told.path === 'string' ? told.path : job.path,
      encoder: typeof told.encoder === 'string' ? told.encoder : job.encoder,
      message: typeof told.message === 'string' ? told.message : job.message,
    };
  }

  /** The engine that rendered the export is gone: it ends there. */
  private stopped(): void {
    if (this.job && (this.job.state === 'running' || this.job.state === 'cancelling')) {
      this.job = { ...this.job, state: 'stopped' };
      this.changed();
    }
  }

  private changed(): void {
    this.snapshot = { job: this.job, asking: this.asking };
    for (const listener of [...this.listeners]) listener();
  }
}

/** Why an export was not started, from the error of the engine or of the shell. */
export function refusalOf(code: number | null, data: unknown, message: string): Extract<RenderStart, { kind: 'refused' }> {
  const reason = data !== null && typeof data === 'object' ? (data as { reason?: unknown }).reason : undefined;
  const path = data !== null && typeof data === 'object' && typeof (data as { path?: unknown }).path === 'string' ? (data as { path: string }).path : '';
  if (code === -32001 && (reason === 'busy' || reason === 'exists' || reason === 'part_exists' || reason === 'source')) {
    return { kind: 'refused', reason, detail: path };
  }
  if (code === -32602) return { kind: 'refused', reason: 'invalid', detail: message };
  return { kind: 'refused', reason: 'other', detail: message };
}

/** Whether a film came out as ProRes in a .mov where H.264 or HEVC was asked for: the computer has no encoder for them. */
export function fellBack(job: Pick<RenderJob, 'codec' | 'encoder' | 'state'>): boolean {
  return job.state === 'done' && job.codec !== 'prores' && (job.encoder ?? '').startsWith('prores');
}

const made = new WeakMap<object, Renders>();

/** The exports of the page, for the engine of the studio that the bridge reaches. */
export function rendersOf(bridge: Pick<EngineBridge, 'call' | 'onEvent' | 'render' | 'reveal'>): Renders {
  let renders = made.get(bridge);
  if (!renders) {
    renders = new Renders(bridge);
    made.set(bridge, renders);
  }
  return renders;
}
