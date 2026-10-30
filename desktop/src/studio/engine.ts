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
  /** A decoded still holds its frame for the entire clip. */
  still?: boolean;
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
  name: string;
  bin: string | null;
  mark_in: Time | null;
  mark_out: Time | null;
  path: string;
  identity: Identity;
  facts: Facts;
  proxy?: Proxy;
}

export type TrackKind = 'video' | 'audio' | 'subtitle';
export type Monitor = 'source' | 'timeline';
export interface Bin { id: string; name: string; parent: string | null }
export interface Speed { num: number; den: number }
export interface Transform { x: number; y: number; scale: number; rotation: number; opacity: number }
export interface TitleStyle { font: string; size: number; color: string; x: number; y: number; align: 'left' | 'center' | 'right' }
export type ClipSource =
  | { kind: 'media'; media: string; in: Time }
  | { kind: 'title'; text: string; style: TitleStyle }
  | { kind: 'color'; color: string };
export interface Clip {
  id: string;
  start: number;
  length: number;
  source: ClipSource;
  enabled: boolean;
  speed: Speed;
  reverse: boolean;
  link: string | null;
  transform: Transform;
  gain_db: number;
  pan: number;
  fade_in: number;
  fade_out: number;
}
export type TransitionKind = 'dissolve' | 'dip_black' | 'dip_white' | 'crossfade';
export type Alignment = 'start' | 'center' | 'end';
export interface Transition { id: string; kind: TransitionKind; left: string; right: string; length: number; align: Alignment }
export interface Cue { id: string; start: number; length: number; text: string }
interface TrackBase { id: string; name: string; enabled: boolean; locked: boolean }
export interface ClipTrack extends TrackBase { kind: 'video' | 'audio'; clips: Clip[]; transitions: Transition[] }
export interface SubtitleTrack extends TrackBase { kind: 'subtitle'; language: string; cues: Cue[] }
export type Track = ClipTrack | SubtitleTrack;
export interface Timeline {
  id: string;
  name: string;
  rate: Rate;
  width: number;
  height: number;
  sample_rate: number;
  tracks: Track[];
}
export interface Project { schema: 2; next_id: number; bins: Bin[]; media: Media[]; timelines: Timeline[] }

type Settings<T, K extends keyof T> = { [P in K]?: T[P] | null };
/** Basic operations broadcast in `done`. Editing commands are expanded by the engine alone. */
export type Op =
  | { op: 'add_media'; media: Media; index?: number | null }
  | { op: 'remove_media'; id: string }
  | ({ op: 'set_media'; id: string; still?: boolean | null } & Settings<Media, 'name' | 'bin' | 'mark_in' | 'mark_out'>)
  | { op: 'set_proxy'; media: string; proxy: Proxy | null }
  | { op: 'add_bin'; bin: Bin; index?: number | null }
  | { op: 'remove_bin'; id: string }
  | ({ op: 'set_bin'; id: string } & Settings<Bin, 'name' | 'parent'>)
  | { op: 'add_timeline'; timeline: Timeline; index?: number | null }
  | { op: 'remove_timeline'; id: string }
  | ({ op: 'set_timeline'; id: string } & Settings<Timeline, 'name' | 'rate' | 'width' | 'height' | 'sample_rate'>)
  | { op: 'add_track'; timeline: string; track: Track; index?: number | null }
  | { op: 'remove_track'; timeline: string; id: string }
  | ({ op: 'set_track'; timeline: string; id: string; language?: string | null } & Settings<Track, 'name' | 'enabled' | 'locked'>)
  | { op: 'insert_clip'; timeline: string; track: string; clip: Clip }
  | { op: 'remove_clip'; timeline: string; id: string }
  | { op: 'move_clip'; timeline: string; id: string; track: string; start: number }
  | { op: 'shift_clips'; timeline: string; track: string; clips: string[]; by: number }
  | { op: 'shift_cues'; timeline: string; track: string; cues: string[]; by: number }
  | { op: 'trim_clip'; timeline: string; id: string; source_in?: Time | null; start: number; length: number }
  | ({ op: 'set_clip'; timeline: string; id: string } & Settings<Clip, 'enabled' | 'speed' | 'reverse' | 'link' | 'transform' | 'gain_db' | 'pan' | 'fade_in' | 'fade_out' | 'source'>)
  | { op: 'add_transition'; timeline: string; track: string; transition: Transition }
  | { op: 'remove_transition'; timeline: string; id: string }
  | ({ op: 'set_transition'; timeline: string; id: string } & Settings<Transition, 'kind' | 'length' | 'align'>)
  | { op: 'add_cue'; timeline: string; track: string; cue: Cue }
  | { op: 'remove_cue'; timeline: string; id: string }
  | ({ op: 'set_cue'; timeline: string; id: string } & Settings<Cue, 'start' | 'length' | 'text'>);

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
interface TransportBase {
  frame: number | null;
  playing: boolean;
  speed: number;
  exact: boolean;
  shown: number | null;
  last: number | null;
  sound: 'none' | 'on' | 'off' | 'unavailable';
  proxies: boolean;
}
export interface SourceTransportState extends TransportBase {
  monitor: 'source';
  media: string | null;
  source: 'proxy' | 'original' | null;
}
export interface TimelineTransportState extends TransportBase {
  monitor: 'timeline';
  timeline: string | null;
  source: 'proxy' | 'original' | 'mixed' | null;
  late: number;
}
export type TransportState = SourceTransportState | TimelineTransportState;

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
  /** Input edit indexes whose requested amount exceeded the available handles. */
  clamped: number[];
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

/** Export options from the page. Only the native Save As dialog supplies the output path. */
export interface RenderOptions {
  timeline: string;
  codec: 'h264' | 'hevc' | 'prores';
  quality: 'high' | 'medium' | 'small';
  range?: { from: number; to: number };
  subtitles: boolean;
  language?: string;
  /** The name of the timeline, which Save As suggests to name the file after. */
  name?: string;
}
export type RenderStarted = { job: string } | { cancelled: true };

/** What the engine tells of an export as it goes, at most every 250 ms, and once at its end (`render.progress`). */
export interface RenderProgress {
  job: string;
  state: 'running' | 'done' | 'failed' | 'cancelled';
  /** Frames done, and of how many. */
  frame: number;
  frames: number;
  fps: number;
  /** Where the film is written: another file than the one chosen where ProRes was written instead. */
  path?: string;
  /** The encoder of libavcodec, once one is open, such as `h264_nvenc` or `prores_ks`. */
  encoder?: string;
  /** Why it failed, that it fell back to ProRes, or that the graphics device was made again. */
  message?: string;
}

/**
 * What became of one old project of the folder as the studio was opened: the timeline that it
 * became and what of it was left out, or why it could not be migrated (`error`).
 */
export interface MigrationReport {
  /** The folder of the old project. */
  source: string;
  timeline?: string;
  dropped?: string[];
  warnings?: string[];
  error?: string;
}

/** The methods of the engine that the page may call, and what each answers. */
export interface PageMethods {
  'project.get': ProjectGot;
  'transaction.apply': Applied;
  'history.undo': Stepped;
  'history.redo': Stepped;
  'media.make_proxy': Record<string, never>;
  'timeline.duration': { frames: number };
  'media.thumbnail': { width: number; height: number; png: string };
  'media.waveform': { rate: number; channels: number; peaks: string } | { pending: true };
  'render.cancel': Record<string, never>;
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
  | { kind: 'attached'; session: number; monitor: Monitor; adapter: string | null; backend: string | null }
  | { kind: 'unsupported'; session: number; monitor: Monitor; reason: 'unsupported' | 'headless'; message: string }
  /** A headless engine (an agent's) had the project, and was asked to yield it; `opened` follows. */
  | { kind: 'taking-over'; session: number; pid: number | null }
  /**
   * The old projects of the folder were looked at once the engine was open: those that became
   * timelines and those that could not, or why none could be looked at (`error`). Told only where
   * there was something to say.
   */
  | { kind: 'migrated'; session: number; migrations: MigrationReport[]; error: string | null }
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

/** The engine sorts clips/cues by start, transitions by their right clip's cut. */
function ordered(track: Track): Track {
  if (track.kind === 'subtitle') return { ...track, cues: [...track.cues].sort((a, b) => a.start - b.start) };
  const starts = new Map(track.clips.map(clip => [clip.id, clip.start]));
  return { ...track, clips: [...track.clips].sort((a, b) => a.start - b.start),
    transitions: [...track.transitions].sort((a, b) => (starts.get(a.right) ?? Infinity) - (starts.get(b.right) ?? Infinity)) };
}

function clipAt(timeline: Timeline, id: string): [number, number] {
  for (let index = 0; index < timeline.tracks.length; index += 1) {
    const track = timeline.tracks[index];
    if (track.kind === 'subtitle') continue;
    const at = track.clips.findIndex(clip => clip.id === id);
    if (at >= 0) return [index, at];
  }
  throw new CopyMismatch(`there is no clip ${id} in the copy`);
}

function inTimeline(project: Project, id: string, change: (timeline: Timeline) => Timeline): Project {
  const at = placeOf(project.timelines, id, 'timeline');
  return { ...project, timelines: replaced(project.timelines, at, change(project.timelines[at])) };
}

function inTrack(timeline: Timeline, at: number, change: (track: Track) => Track): Timeline {
  return { ...timeline, tracks: replaced(timeline.tracks, at, change(timeline.tracks[at])) };
}

function clipTrack(track: Track): ClipTrack {
  if (track.kind === 'subtitle') throw new CopyMismatch(`track ${track.id} holds subtitles, not clips`);
  return track;
}

function inClips(timeline: Timeline, at: number, change: (clips: Clip[]) => Clip[]): Timeline {
  return inTrack(timeline, at, track => ordered({ ...clipTrack(track), clips: change(clipTrack(track).clips) }));
}

/** Only double-option fields distinguish null (clear it) from an absent field (keep it). */
function patched<T>(item: T, op: object, keys: readonly string[], nullable: readonly string[] = []): T {
  const fields = op as Record<string, unknown>;
  const changed = Object.fromEntries(keys.filter(key => fields[key] !== undefined && (fields[key] !== null || nullable.includes(key))).map(key => [key, fields[key]]));
  return { ...item, ...changed };
}

function inCue(timeline: Timeline, id: string, change: (cues: Cue[], at: number) => Cue[]): Timeline {
  for (let i = 0; i < timeline.tracks.length; i += 1) {
    const track = timeline.tracks[i];
    if (track.kind !== 'subtitle') continue;
    const at = track.cues.findIndex(cue => cue.id === id);
    if (at >= 0) return inTrack(timeline, i, () => ordered({ ...track, cues: change(track.cues, at) }));
  }
  throw new CopyMismatch(`there is no subtitle ${id} in the copy`);
}

function inTransition(timeline: Timeline, id: string, change: (items: Transition[], at: number) => Transition[]): Timeline {
  for (let i = 0; i < timeline.tracks.length; i += 1) {
    const track = timeline.tracks[i];
    if (track.kind === 'subtitle') continue;
    const at = track.transitions.findIndex(item => item.id === id);
    if (at >= 0) return inTrack(timeline, i, () => ordered({ ...track, transitions: change(track.transitions, at) }));
  }
  throw new CopyMismatch(`there is no transition ${id} in the copy`);
}

/** Mirror only accepted basic operations, preserving all objects outside the changed branch. */
export function applyOp(project: Project, op: Op): Project {
  switch (op.op) {
    case 'add_media': return { ...project, media: inserted(project.media, op.media, op.index) };
    case 'remove_media': return { ...project, media: without(project.media, placeOf(project.media, op.id, 'media')) };
    case 'set_media': {
      const at = placeOf(project.media, op.id, 'media');
      let media = patched(project.media[at], op, ['name', 'bin', 'mark_in', 'mark_out'], ['bin', 'mark_in', 'mark_out']);
      if (op.still != null) {
        if (!media.facts.video) throw new CopyMismatch(`media ${op.id} has no picture`);
        const { still: _old, ...picture } = media.facts.video;
        media = { ...media, facts: { ...media.facts, video: op.still ? { ...picture, still: true } : picture } };
      }
      return { ...project, media: replaced(project.media, at, media) };
    }
    case 'set_proxy': {
      const at = placeOf(project.media, op.media, 'media');
      const { proxy: _before, ...media } = project.media[at];
      return { ...project, media: replaced(project.media, at, op.proxy == null ? media : { ...media, proxy: op.proxy }) };
    }
    case 'add_bin': return { ...project, bins: inserted(project.bins, op.bin, op.index) };
    case 'remove_bin': return { ...project, bins: without(project.bins, placeOf(project.bins, op.id, 'bin')) };
    case 'set_bin': {
      const at = placeOf(project.bins, op.id, 'bin');
      return { ...project, bins: replaced(project.bins, at, patched(project.bins[at], op, ['name', 'parent'], ['parent'])) };
    }
    case 'add_timeline': return { ...project, timelines: inserted(project.timelines, { ...op.timeline, tracks: op.timeline.tracks.map(ordered) }, op.index) };
    case 'remove_timeline': return { ...project, timelines: without(project.timelines, placeOf(project.timelines, op.id, 'timeline')) };
    case 'set_timeline': return inTimeline(project, op.id, timeline => patched(timeline, op, ['name', 'rate', 'width', 'height', 'sample_rate']));
    case 'add_track': return inTimeline(project, op.timeline, timeline => ({ ...timeline, tracks: inserted(timeline.tracks, ordered(op.track), op.index) }));
    case 'remove_track': return inTimeline(project, op.timeline, timeline => ({ ...timeline, tracks: without(timeline.tracks, placeOf(timeline.tracks, op.id, 'track')) }));
    case 'set_track': return inTimeline(project, op.timeline, timeline => inTrack(timeline, placeOf(timeline.tracks, op.id, 'track'), track => patched(track, op, ['name', 'enabled', 'locked', ...(track.kind === 'subtitle' ? ['language'] : [])])));
    case 'insert_clip': return inTimeline(project, op.timeline, timeline => inClips(timeline, placeOf(timeline.tracks, op.track, 'track'), clips => put(clips, op.clip)));
    case 'remove_clip': return inTimeline(project, op.timeline, timeline => {
      const [track, at] = clipAt(timeline, op.id);
      return inClips(timeline, track, clips => without(clips, at));
    });
    case 'move_clip': return inTimeline(project, op.timeline, timeline => {
      const [from, at] = clipAt(timeline, op.id);
      const to = placeOf(timeline.tracks, op.track, 'track');
      const moved = { ...clipTrack(timeline.tracks[from]).clips[at], start: op.start };
      const taken = inClips(timeline, from, clips => without(clips, at));
      return inClips(taken, to, clips => put(clips, moved));
    });
    case 'shift_clips': return inTimeline(project, op.timeline, timeline => {
      const at = placeOf(timeline.tracks, op.track, 'track');
      const ids = new Set(op.clips);
      return inClips(timeline, at, clips => {
        for (const id of ids) placeOf(clips, id, 'clip');
        return clips.map(clip => ids.has(clip.id) ? { ...clip, start: clip.start + op.by } : clip);
      });
    });
    case 'trim_clip': return inTimeline(project, op.timeline, timeline => {
      const [track, at] = clipAt(timeline, op.id);
      return inClips(timeline, track, clips => {
        const clip = clips[at];
        const source = clip.source.kind === 'media' && op.source_in != null ? { ...clip.source, in: op.source_in } : clip.source;
        return put(without(clips, at), { ...clip, source, start: op.start, length: op.length });
      });
    });
    case 'set_clip': return inTimeline(project, op.timeline, timeline => {
      const [track, at] = clipAt(timeline, op.id);
      return inClips(timeline, track, clips => replaced(clips, at, patched(clips[at], op,
        ['enabled', 'speed', 'reverse', 'link', 'transform', 'gain_db', 'pan', 'fade_in', 'fade_out', 'source'], ['link'])));
    });
    case 'add_transition': return inTimeline(project, op.timeline, timeline => inTrack(timeline, placeOf(timeline.tracks, op.track, 'track'), track => {
      const to = clipTrack(track);
      return ordered({ ...to, transitions: [...to.transitions, op.transition] });
    }));
    case 'remove_transition': return inTimeline(project, op.timeline, timeline => inTransition(timeline, op.id, (items, at) => without(items, at)));
    case 'set_transition': return inTimeline(project, op.timeline, timeline => inTransition(timeline, op.id, (items, at) => replaced(items, at, patched(items[at], op, ['kind', 'length', 'align']))));
    case 'add_cue': return inTimeline(project, op.timeline, timeline => inTrack(timeline, placeOf(timeline.tracks, op.track, 'track'), track => {
      if (track.kind !== 'subtitle') throw new CopyMismatch(`track ${track.id} holds no subtitles`);
      return ordered({ ...track, cues: [...track.cues, op.cue] });
    }));
    case 'shift_cues': return inTimeline(project, op.timeline, timeline => inTrack(timeline, placeOf(timeline.tracks, op.track, 'track'), track => {
      if (track.kind !== 'subtitle') throw new CopyMismatch(`track ${track.id} holds no subtitles`);
      const ids = new Set(op.cues);
      for (const id of ids) placeOf(track.cues, id, 'cue');
      return ordered({ ...track, cues: track.cues.map(cue => ids.has(cue.id) ? { ...cue, start: cue.start + op.by } : cue) });
    }));
    case 'remove_cue': return inTimeline(project, op.timeline, timeline => inCue(timeline, op.id, (items, at) => without(items, at)));
    case 'set_cue': return inTimeline(project, op.timeline, timeline => inCue(timeline, op.id, (items, at) => replaced(items, at, patched(items[at], op, ['start', 'length', 'text']))));
    default: throw new CopyMismatch(`the operation ${(op as { op: unknown }).op} is not known to the copy`);
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
  transports: { source: SourceTransportState | null; timeline: TimelineTransportState | null };
  /** How the proxy of each media item is being made, as the engine told last, by the media. */
  proxies: Readonly<Record<string, ProxyEvent>>;
  views: Record<Monitor, ViewState>;
  /** What went wrong last: an error of the engine, or a fetch that failed. Null where nothing did. */
  error: string | null;
  /** Whether the headless engine of an agent is being asked to yield the project, until it is opened. */
  takingOver: boolean;
}

const NOTHING: StudioSnapshot = {
  status: 'disconnected', session: null, revision: null, project: null, transports: { source: null, timeline: null }, proxies: {}, views: { source: { state: 'none' }, timeline: { state: 'none' } },
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
            session: event.session, status: 'disconnected', revision: null, project: null, transports: { source: null, timeline: null }, proxies: {},
            views: { source: { state: 'none' }, timeline: { state: 'none' } }, error: null, takingOver: false,
          });
        } else if (this.snapshot.takingOver) this.set({ takingOver: false });
        void this.refetch();
        return;
      case 'exited':
        this.epoch += 1;
        this.open = false;
        this.fetchAgain = false;
        this.cancelRetry();
        this.set({ status: 'disconnected', transports: { source: null, timeline: null }, views: { source: { state: 'none' }, timeline: { state: 'none' } } });
        return;
      case 'taking-over':
        this.set({ takingOver: true });
        return;
      case 'error':
        // An engine that could not be opened is taken over no more.
        this.set({ error: event.message, takingOver: false });
        return;
      case 'attached':
        this.set({ views: { ...this.snapshot.views, [event.monitor]: { state: 'attached', adapter: event.adapter, backend: event.backend } } });
        return;
      case 'unsupported':
        this.set({ views: { ...this.snapshot.views, [event.monitor]: { state: 'unsupported', reason: event.reason } } });
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
    else if (method === 'transport.state') {
      const state = params as TransportState;
      if (state.monitor === 'source' || state.monitor === 'timeline') {
        this.set({ transports: { ...this.snapshot.transports, [state.monitor]: state } });
      }
    }
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
    let at = 0;
    try {
      let next: Project = { ...project, next_id: change.next_id };
      for (const op of change.done) { next = applyOp(next, op); at++; }
      this.set({ revision: change.revision, project: next });
    } catch (error) {
      // The copy and the engine disagree (or the page does not know an operation): it is fetched
      // again whole. This should not happen, so it is said, with the operation that failed.
      console.warn(`The copy of the project could not apply ${change.done[at]?.op ?? 'a change'} (operation ${at + 1} of revision ${change.revision}); it is fetched again.`, error);
      void this.refetch();
    }
  }
}
