/**
 * The engine of the studio as the page sees it: the project as `project.get` gives it, the
 * operations that change it, and what the engine tells. See engine/PROTOCOL.md.
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
  | { kind: 'unsupported'; session: number; reason: 'unsupported' | 'headless'; message: string };

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
