/**
 * The engine of the studio as the page sees it: the project as `project.get` gives it, the
 * operations that change it, what the engine tells, and the page's own copy of the project,
 * kept as the engine tells of every change. See engine/PROTOCOL.md.
 *
 * The page reaches the engine only through the shell (`window.columbina.engine`): it names no
 * path to it, and holds no handle of the window.
 */
import type { EngineBridge } from '../bridge';

/** A rate of frames, as a fraction: 24000/1001 is 23.976 frames a second. */
export interface Rate {
  num: number;
  den: number;
}

/** A place in a source: a count of ticks, each `1/scale` of a second. */
export interface Time {
  ticks: number;
  scale: number;
}

/** What tells a file from another one at the same path. */
export interface Identity {
  size: number;
  modified_us: number;
  digest: string;
}

/** The picture of a file. */
export interface VideoFacts {
  width: number;
  height: number;
  rate: Rate;
  codec: string;
  bits: number;
  /** How far the picture is turned clockwise to be shown upright, in degrees. */
  rotation: number;
}

/** The sound of a file. */
export interface AudioFacts {
  rate: number;
  channels: number;
  codec: string;
}

/** What a file holds. A file without a picture has no `video`, one without sound no `audio`. */
export interface Facts {
  duration: Time;
  video?: VideoFacts | null;
  audio?: AudioFacts | null;
}

/** A copy of the picture of media whose every frame is a key frame, made by the engine. */
export interface Proxy {
  path: string;
  width: number;
  height: number;
  codec: string;
  digest_of_source: string;
}

/** A file that the project is cut from. `proxy` is left out where it has none. */
export interface Media {
  id: string;
  path: string;
  identity: Identity;
  facts: Facts;
  proxy?: Proxy;
}

export type TrackKind = 'video' | 'audio';

/** A stretch of a source on a track: from `source_in` in the source, at frame `start`, for `length` frames. */
export interface Clip {
  id: string;
  media: string;
  source_in: Time;
  start: number;
  length: number;
}

/** A track of a timeline, and its clips in the order of their start. */
export interface Track {
  id: string;
  kind: TrackKind;
  clips: Clip[];
}

/** A timeline: tracks of clips, laid on frames of one rate. */
export interface Timeline {
  id: string;
  name: string;
  rate: Rate;
  tracks: Track[];
}

/** A project of the studio, as `project.json` and `project.get` have it. */
export interface Project {
  schema: number;
  next_id: number;
  media: Media[];
  timelines: Timeline[];
}

/** A change to a project. A missing `index` means "at the end". */
export type Op =
  | { op: 'add_media'; media: Media; index?: number | null }
  | { op: 'remove_media'; id: string }
  | { op: 'add_timeline'; timeline: Timeline; index?: number | null }
  | { op: 'remove_timeline'; id: string }
  | { op: 'add_track'; timeline: string; track: Track; index?: number | null }
  | { op: 'remove_track'; timeline: string; id: string }
  | { op: 'insert_clip'; timeline: string; track: string; clip: Clip }
  | { op: 'remove_clip'; timeline: string; id: string }
  | { op: 'move_clip'; timeline: string; id: string; track: string; start: number }
  | { op: 'trim_clip'; timeline: string; id: string; source_in: Time; start: number; length: number }
  | { op: 'set_proxy'; media: string; proxy: Proxy | null };

/** Who asked for a change, or made the step that it takes back or does again. */
export type ClientKind = 'desktop' | 'agent' | 'cli' | 'engine';

/** The notification `project.changed`. */
export interface ProjectChanged {
  revision: number;
  by: ClientKind;
  made_by: ClientKind;
  done: Op[];
  next_id: number;
}

/** The state of the transport: the result of `transport.load` and `transport.state`, and the notification `transport.state`. */
export interface TransportState {
  media: string | null;
  frame: number | null;
  playing: boolean;
  speed: number;
  exact: boolean;
  shown: number | null;
  last: number | null;
  sound: 'none' | 'on' | 'off' | 'unavailable';
  source: 'proxy' | 'original' | null;
  proxies: boolean;
}

/** The notification `media.proxy`: how the proxy of media is being made. */
export interface ProxyEvent {
  media: string;
  state: 'queued' | 'running' | 'done' | 'failed';
  progress?: number;
  message?: string;
}

/** The notification `engine.error`. */
export interface EngineErrorNotice {
  message: string;
  kind: string;
}

/** The result of `project.get`. */
export interface ProjectGot {
  revision: number;
  next_id: number;
  project: Project;
}

/** The result of `transaction.apply`: the operations as they were done, and the ids that `$names` were given. */
export interface Applied {
  revision: number;
  done: Op[];
  next_id: number;
  names: Record<string, string>;
}

/** The result of `history.undo` and `history.redo`: `done` is null where there was nothing to take back or do again. */
export interface Stepped {
  revision: number;
  done: Op[] | null;
  next_id?: number;
}

/** The result of `media.import`: the media added, in the order of the files. */
export interface Imported {
  revision: number;
  added: string[];
  next_id: number;
}

/** The methods of the engine that the page may call, and what each answers. */
export interface PageMethods {
  'project.get': ProjectGot;
  'transaction.apply': Applied;
  'history.undo': Stepped;
  'history.redo': Stepped;
  'media.make_proxy': Record<string, never>;
  'transport.load': TransportState;
  'transport.play': Record<string, never>;
  'transport.pause': Record<string, never>;
  'transport.seek': Record<string, never>;
  'transport.step': Record<string, never>;
  'transport.state': TransportState;
  'transport.proxies': Record<string, never>;
  'view.place': Record<string, never>;
  'view.show': Record<string, never>;
}
export type PageMethod = keyof PageMethods;

/** Where the viewer lies: device pixels of the screen, from the top left corner of the client area. */
export interface ViewRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** What the shell answers when a project is opened in the studio. */
export interface EngineOpened {
  /** The number of this engine's session; every event of it carries it. */
  session: number;
  /** The directory of the project of the studio. */
  project: string;
  /** Whether this application started the engine, or reached one that had the project open. */
  owned: boolean;
  pid: number;
  revision: number;
}

/** What the shell tells the page of the engine, in the order it happened. */
export type EngineEvent =
  | ({ kind: 'opened' } & EngineOpened)
  | { kind: 'notification'; session: number; method: string; params: unknown }
  | { kind: 'exited'; session: number; expected: boolean; code: number | null; signal: string | null }
  | { kind: 'error'; session: number; message: string; code?: number | null; data?: unknown }
  | { kind: 'attached'; session: number; adapter: string | null; backend: string | null }
  | { kind: 'unsupported'; session: number; reason: 'unsupported' | 'headless'; message: string }
  /** A headless engine (an agent's) had the project, and was asked to yield it; `opened` follows. */
  | { kind: 'taking-over'; session: number; pid: number | null }
  /**
   * The pixels of the screen that the viewer covers changed (the page was zoomed): the page
   * measures the viewer again and says where it is. Told with no session where none is open.
   */
  | { kind: 'measure'; session: number | null };

/** A failure as the engine gave it, or one of the shell's own (code null). */
export interface EngineFailure {
  code: number | null;
  message: string;
  data: unknown;
}

/** What the shell answers to a call of the engine: what the engine answered, or its error whole. */
export type EngineAnswer<T = unknown> = { ok: true; result: T } | { ok: false; error: EngineFailure };

/** An error of the engine, with its code and data: `-32001` for a refused transaction, and so on. */
export class EngineError extends Error {
  readonly code: number | null;
  readonly data: unknown;
  constructor(failure: EngineFailure) {
    super(failure.message);
    this.name = 'EngineError';
    this.code = failure.code;
    this.data = failure.data;
  }
}

/** Call a method of the engine, and get its result, or an `EngineError`. */
export async function callEngine<M extends PageMethod>(
  bridge: Pick<EngineBridge, 'call'>, method: M, params?: Record<string, unknown>,
): Promise<PageMethods[M]> {
  const answer = await bridge.call(method, params);
  if (!answer.ok) throw new EngineError(answer.error);
  return answer.result as PageMethods[M];
}

/**
 * The rectangle of an element in device pixels, as the engine takes it: its left and top rounded,
 * then its right and bottom, and the width and height between them, so that neighbours share
 * their edges. `box` is `getBoundingClientRect()`, and `ratio` is `devicePixelRatio`.
 */
export function deviceRect(box: { left: number; top: number; right: number; bottom: number }, ratio: number): ViewRect {
  const x = Math.max(0, Math.round(box.left * ratio));
  const y = Math.max(0, Math.round(box.top * ratio));
  const right = Math.max(x, Math.round(box.right * ratio));
  const bottom = Math.max(y, Math.round(box.bottom * ratio));
  return { x, y, width: right - x, height: bottom - y };
}

/** A copy that does not hold what the engine says it changed: it is fetched anew. */
export class CopyMismatch extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CopyMismatch';
  }
}

function placeOf<T extends { id: string }>(list: readonly T[], id: string, what: string): number {
  const at = list.findIndex(thing => thing.id === id);
  if (at < 0) throw new CopyMismatch(`there is no ${what} ${id} in the copy`);
  return at;
}

function inserted<T>(list: readonly T[], thing: T, index: number | null | undefined): T[] {
  const at = index ?? list.length;
  if (!Number.isInteger(at) || at < 0 || at > list.length) throw new CopyMismatch(`index ${at} is past the end of a list of ${list.length}`);
  return [...list.slice(0, at), thing, ...list.slice(at)];
}

function without<T>(list: readonly T[], at: number): T[] {
  return [...list.slice(0, at), ...list.slice(at + 1)];
}

function replaced<T>(list: readonly T[], at: number, thing: T): T[] {
  return list.map((each, index) => (index === at ? thing : each));
}

/** Put a clip among the clips of a track, in the order of their start: before the first that starts no earlier. */
function put(clips: readonly Clip[], clip: Clip): Clip[] {
  const at = clips.findIndex(each => !(each.start < clip.start));
  return inserted(clips, clip, at < 0 ? clips.length : at);
}

/** A track whose clips are in the order of their start, as the engine keeps them. */
function ordered(track: Track): Track {
  return { ...track, clips: [...track.clips].sort((one, other) => one.start - other.start) };
}

/** Where a clip is in a timeline: its track, and its place on it. */
function clipAt(timeline: Timeline, id: string): [number, number] {
  for (let track = 0; track < timeline.tracks.length; track += 1) {
    const at = timeline.tracks[track].clips.findIndex(clip => clip.id === id);
    if (at >= 0) return [track, at];
  }
  throw new CopyMismatch(`there is no clip ${id} in the copy`);
}

function inTimeline(project: Project, id: string, change: (timeline: Timeline) => Timeline): Project {
  const at = placeOf(project.timelines, id, 'timeline');
  return { ...project, timelines: replaced(project.timelines, at, change(project.timelines[at])) };
}

function inTrack(timeline: Timeline, at: number, change: (clips: Clip[]) => Clip[]): Timeline {
  const track = timeline.tracks[at];
  return { ...timeline, tracks: replaced(timeline.tracks, at, { ...track, clips: change(track.clips) }) };
}

/**
 * Apply an operation as the engine did it (from `done`, with every id and place filled in) to a
 * copy of the project, as `crates/model/src/ops.rs` applies it. The project given is not changed:
 * what changed is new, and what did not is shared. Throws `CopyMismatch` where the copy does not
 * hold what the operation names.
 */
export function applyOp(project: Project, op: Op): Project {
  switch (op.op) {
    case 'add_media':
      return { ...project, media: inserted(project.media, op.media, op.index) };
    case 'remove_media':
      return { ...project, media: without(project.media, placeOf(project.media, op.id, 'media')) };
    case 'set_proxy': {
      const at = placeOf(project.media, op.media, 'media');
      const { proxy: _before, ...media } = project.media[at];
      // A proxy taken away is left out, as the engine writes it; one left out of the operation is taken away.
      return { ...project, media: replaced(project.media, at, op.proxy == null ? media : { ...media, proxy: op.proxy }) };
    }
    case 'add_timeline':
      return { ...project, timelines: inserted(project.timelines, { ...op.timeline, tracks: op.timeline.tracks.map(ordered) }, op.index) };
    case 'remove_timeline':
      return { ...project, timelines: without(project.timelines, placeOf(project.timelines, op.id, 'timeline')) };
    case 'add_track':
      return inTimeline(project, op.timeline, timeline => ({ ...timeline, tracks: inserted(timeline.tracks, ordered(op.track), op.index) }));
    case 'remove_track':
      return inTimeline(project, op.timeline, timeline => ({ ...timeline, tracks: without(timeline.tracks, placeOf(timeline.tracks, op.id, 'track')) }));
    case 'insert_clip':
      return inTimeline(project, op.timeline, timeline => inTrack(timeline, placeOf(timeline.tracks, op.track, 'track'), clips => put(clips, op.clip)));
    case 'remove_clip':
      return inTimeline(project, op.timeline, (timeline) => {
        const [track, at] = clipAt(timeline, op.id);
        return inTrack(timeline, track, clips => without(clips, at));
      });
    case 'move_clip':
      return inTimeline(project, op.timeline, (timeline) => {
        const [from, at] = clipAt(timeline, op.id);
        const to = placeOf(timeline.tracks, op.track, 'track');
        const moved = { ...timeline.tracks[from].clips[at], start: op.start };
        const taken = inTrack(timeline, from, clips => without(clips, at));
        return inTrack(taken, to, clips => put(clips, moved));
      });
    case 'trim_clip':
      return inTimeline(project, op.timeline, (timeline) => {
        const [track, at] = clipAt(timeline, op.id);
        const trimmed = { ...timeline.tracks[track].clips[at], source_in: op.source_in, start: op.start, length: op.length };
        return inTrack(timeline, track, clips => put(without(clips, at), trimmed));
      });
    default:
      throw new CopyMismatch(`the operation ${(op as { op: unknown }).op} is not known to the copy`);
  }
}

/** Apply a change that the engine told of: its `next_id` first, then each of its operations in order. */
export function applyChange(project: Project, change: Pick<ProjectChanged, 'done' | 'next_id'>): Project {
  let changed: Project = { ...project, next_id: change.next_id };
  for (const op of change.done) changed = applyOp(changed, op);
  return changed;
}

/** Whether the copy is kept by an engine that answers. */
export type CopyStatus = 'connected' | 'disconnected';

/** How the view of the engine stands in the window. */
export type ViewState =
  | { state: 'none' }
  | { state: 'attached'; adapter: string | null; backend: string | null }
  | { state: 'unsupported'; reason: 'unsupported' | 'headless' };

/** What the page knows of the engine at one moment. A new object whenever anything in it changed. */
export interface StudioSnapshot {
  status: CopyStatus;
  /** The session of the engine that the copy was fetched from, or null before any was opened. */
  session: number | null;
  /** The revision of the copy, or null before it was fetched. */
  revision: number | null;
  /**
   * The copy of the project, kept (as it was last) while disconnected; null before it was fetched,
   * and from the moment another engine is opened until its project is.
   */
  project: Project | null;
  /** What the transport told last, or null before it told anything. */
  transport: TransportState | null;
  /** How the proxy of each media item is being made, as the engine told last, by the media. */
  proxies: Readonly<Record<string, ProxyEvent>>;
  view: ViewState;
  /** What went wrong last: an error of the engine, or a fetch that failed. Null where nothing did. */
  error: string | null;
  /** Whether the headless engine of an agent is being asked to yield the project, until it is opened. */
  takingOver: boolean;
}

const NOTHING: StudioSnapshot = {
  status: 'disconnected', session: null, revision: null, project: null, transport: null, proxies: {}, view: { state: 'none' },
  error: null, takingOver: false,
};

/** How long the copy waits before it fetches the project again after a fetch failed: twice as long each time, up to 5 s. */
const RETRY_FIRST_MS = 250;
const RETRY_MOST_MS = 5_000;

/**
 * The page's copy of the project, kept as the engine tells it.
 *
 * It fetches the project with `project.get` whenever an engine is opened (also again, as after the
 * page was loaded again), and then applies each `project.changed` whose revision is one above its
 * own. One at or below its own is passed over; one that skips a revision, or that the copy cannot
 * apply, makes it fetch the project again. What is told while it fetches is applied after the
 * fetch, from the revision that the fetch answered. A fetch that fails is tried again, a little
 * later each time, while the engine is open. It also keeps what the transport, the proxies and the
 * view told last.
 *
 * It is a store for `useSyncExternalStore(copy.subscribe, copy.getSnapshot)`.
 */
export class ProjectCopy {
  private snapshot: StudioSnapshot = NOTHING;
  private readonly listeners = new Set<() => void>();
  private fetching: Promise<void> | null = null;
  private fetchAgain = false;
  /** What was told while the project was being fetched. */
  private held: ProjectChanged[] = [];
  /** Counted up whenever an engine is opened or ends, so that a fetch of an earlier one is let go. */
  private epoch = 0;
  /** Whether an engine is open, as the shell told: one was opened, and has not ended. */
  private open = false;
  /** How often a fetch failed in a row, and the fetch that is waited for after it. */
  private failures = 0;
  private retry: ReturnType<typeof setTimeout> | null = null;
  private readonly unsubscribe: () => void;

  constructor(private readonly bridge: Pick<EngineBridge, 'call' | 'onEvent'>) {
    this.unsubscribe = bridge.onEvent(event => this.heard(event));
  }

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  readonly getSnapshot = (): StudioSnapshot => this.snapshot;

  /** Stop listening to the engine. The snapshot stays as it is. */
  dispose(): void {
    this.unsubscribe();
    this.listeners.clear();
    this.open = false;
    this.cancelRetry();
  }

  /** Fetch the project anew. A fetch asked for while one is under way is made after it. */
  refetch(): Promise<void> {
    if (this.fetching) {
      this.fetchAgain = true;
      return this.fetching;
    }
    this.cancelRetry();
    this.held = [];
    const epoch = this.epoch;
    this.fetching = (async () => {
      try {
        const got = await callEngine(this.bridge, 'project.get');
        if (epoch !== this.epoch) return;
        let revision = got.revision;
        let project: Project = { ...got.project, next_id: got.next_id };
        for (const change of this.held) {
          if (change.revision <= revision) continue;
          if (change.revision !== revision + 1) {
            this.fetchAgain = true;
            break;
          }
          try {
            project = applyChange(project, change);
          } catch {
            this.fetchAgain = true;
            break;
          }
          revision = change.revision;
        }
        this.failures = 0;
        this.set({ status: 'connected', revision, project, error: null });
      } catch (error) {
        if (epoch !== this.epoch) return;
        this.set({ status: 'disconnected', error: error instanceof Error ? error.message : String(error) });
        // The engine is open, and answers again in a moment: a project that is long to read, or a
        // connection that is made again.
        if (this.open && !this.fetchAgain) {
          const wait = Math.min(RETRY_MOST_MS, RETRY_FIRST_MS * 2 ** this.failures);
          this.failures += 1;
          this.retry = setTimeout(() => {
            this.retry = null;
            if (epoch === this.epoch) void this.refetch();
          }, wait);
        }
      } finally {
        this.fetching = null;
        this.held = [];
        if (this.fetchAgain) {
          this.fetchAgain = false;
          void this.refetch();
        }
      }
    })();
    return this.fetching;
  }

  private cancelRetry(): void {
    if (this.retry !== null) clearTimeout(this.retry);
    this.retry = null;
  }

  private set(changed: Partial<StudioSnapshot>): void {
    this.snapshot = { ...this.snapshot, ...changed };
    for (const listener of [...this.listeners]) listener();
  }

  private heard(event: EngineEvent): void {
    // What an engine that was open before the one that is told of last is passed over. A session
    // that comes after it is heard: one that is being opened, or could not be.
    const { session } = this.snapshot;
    if (typeof event.session === 'number' && session !== null && event.session < session) return;
    if (event.kind !== 'opened' && event.kind !== 'taking-over' && event.kind !== 'error'
      && typeof event.session === 'number' && session !== null && event.session !== session) return;
    switch (event.kind) {
      case 'opened':
        this.epoch += 1;
        this.open = true;
        this.failures = 0;
        if (event.session !== session) {
          // Another engine, maybe of another project: nothing of the one before is shown of it.
          this.set({
            session: event.session, status: 'disconnected', revision: null, project: null, transport: null, proxies: {},
            view: { state: 'none' }, error: null, takingOver: false,
          });
        } else if (this.snapshot.takingOver) this.set({ takingOver: false });
        void this.refetch();
        return;
      case 'exited':
        this.epoch += 1;
        this.open = false;
        this.fetchAgain = false;
        this.cancelRetry();
        this.set({ status: 'disconnected', transport: null, view: { state: 'none' } });
        return;
      case 'taking-over':
        this.set({ takingOver: true });
        return;
      case 'error':
        // An engine that could not be opened is taken over no more.
        this.set({ error: event.message, takingOver: false });
        return;
      case 'attached':
        this.set({ view: { state: 'attached', adapter: event.adapter, backend: event.backend } });
        return;
      case 'unsupported':
        this.set({ view: { state: 'unsupported', reason: event.reason } });
        return;
      case 'notification':
        this.told(event.method, event.params);
        return;
      default:
    }
  }

  private told(method: string, params: unknown): void {
    if (params === null || typeof params !== 'object') return;
    if (method === 'project.changed') this.changed(params as ProjectChanged);
    else if (method === 'transport.state') this.set({ transport: params as TransportState });
    else if (method === 'media.proxy') {
      const told = params as ProxyEvent;
      this.set({ proxies: { ...this.snapshot.proxies, [told.media]: told } });
    } else if (method === 'engine.error') this.set({ error: (params as EngineErrorNotice).message });
  }

  private changed(change: ProjectChanged): void {
    if (this.fetching) {
      this.held.push(change);
      return;
    }
    const { status, revision, project } = this.snapshot;
    // Before the copy was fetched, or once its engine ended, a fetch to come takes it in.
    if (status !== 'connected' || revision === null || project === null) return;
    if (change.revision <= revision) return;
    if (change.revision !== revision + 1) {
      void this.refetch();
      return;
    }
    try {
      this.set({ revision: change.revision, project: applyChange(project, change) });
    } catch {
      void this.refetch();
    }
  }
}
