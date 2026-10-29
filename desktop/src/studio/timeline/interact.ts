import type { Clip, Media, Timeline } from '../engine';
import type { Edit, Transaction } from './commands';
import { clipLocations, linkedClips, type Located } from './selection';
import { snap } from './snap';

export type Tool = 'select' | 'trim' | 'blade';
export type Hit = 'move' | 'trim_in' | 'trim_out' | 'slip' | 'slide' | 'blade' | { kind: 'roll'; left: string; right: string };
export interface Preview { id: string; track: string; start: number; length: number }
export interface Gesture {
  timeline: Timeline; revision: number; ids: string[]; clip: string; action: Exclude<Hit, 'blade'>;
  origin: number; linked: boolean; rippleTrim: boolean; locations: Map<string, Located>;
}
export interface Dragged {
  gesture: Gesture; delta: number; point: number | null; clamped: boolean; preview: Preview[]; command: Edit | null;
  /** What a move would overwrite of the clips that stay: parts of them, on their tracks. */
  covered: Preview[];
}

/** A two-pixel seam rolls; the adjoining six-pixel edge trims. Body halves select slip or slide. */
export function hitClip(timeline: Timeline, id: string, tool: Tool, x: number, y: number, width: number, height: number): Hit {
  if (tool === 'blade') return 'blade';
  const here = clipLocations(timeline).get(id);
  if (!here) return 'move';
  const { clip, track, index } = here;
  const previous = track.clips[index - 1];
  const next = track.clips[index + 1];
  const edge = Math.min(6, width / 3);
  if (x <= Math.min(2, edge) && previous && previous.start + previous.length === clip.start) return { kind: 'roll', left: previous.id, right: id };
  if (width - x <= Math.min(2, edge) && next?.start === clip.start + clip.length) return { kind: 'roll', left: id, right: next.id };
  if (x <= edge) return 'trim_in';
  if (width - x <= edge) return 'trim_out';
  return tool === 'trim' ? y < height / 2 ? 'slip' : 'slide' : 'move';
}

export function beginDrag(timeline: Timeline, revision: number, selection: readonly string[], id: string, action: Hit, origin: number, linked: boolean, rippleTrim = false): Gesture | null {
  const locations = clipLocations(timeline);
  const here = locations.get(id);
  if (!here || here.track.locked || action === 'blade') return null;
  const ids = linkedClips(timeline, action === 'move' && selection.includes(id) ? [id, ...selection] : [id], linked);
  return { timeline, revision, ids, clip: id, action, origin, linked, rippleTrim, locations };
}

function handles(clip: Clip, timeline: Timeline, media: readonly Media[]): { before: number; after: number } {
  if (clip.source.kind !== 'media') return { before: Infinity, after: Infinity };
  const source = clip.source;
  const item = media.find(item => item.id === source.media);
  if (item?.facts.video?.still) return { before: Infinity, after: Infinity };
  if (!item) return { before: 0, after: 0 };
  const step = timeline.rate.den / timeline.rate.num * clip.speed.num / clip.speed.den;
  const start = source.in.ticks / source.in.scale;
  const end = item.facts.duration.ticks / item.facts.duration.scale;
  return { before: Math.floor(start / step + 1e-8), after: Math.floor((end - start) / step + 1e-8) - clip.length };
}

/** The free frames before and after a clip on its track: up to the clips beside it that do not move with it. */
function room(here: Located, moving: ReadonlySet<string>): { before: number; after: number } {
  const { clip, track, index } = here;
  let before = clip.start;
  for (let at = index - 1; at >= 0; at--) {
    const other = track.clips[at];
    if (moving.has(other.id)) continue;
    before = clip.start - (other.start + other.length);
    break;
  }
  let after = Infinity;
  for (let at = index + 1; at < track.clips.length; at++) {
    const other = track.clips[at];
    if (moving.has(other.id)) continue;
    after = other.start - (clip.start + clip.length);
    break;
  }
  return { before, after };
}

/**
 * Where a drag puts what it moves, as DaVinci Resolve drags clips in the Selection mode (Reference
 * Manual 20, "Moving, Resizing, and Rolling Clips in Selection Mode" and "Shuffle/Swap Insert
 * Edits"): a move overwrites what it is dropped on (`overwrite`, the default); with Ctrl and Shift
 * it is a shuffle insert (`ripple`), which takes the clips out and puts them in at the place, the
 * rest moving aside; with Shift alone it only changes tracks, keeping its place in time
 * (`vertical`). An edge that is trimmed without ripple stops at the clip beside it.
 *
 * The preview follows source handles, minimum lengths and the clips beside; the engine resolves
 * the rest (ripple, links and its final clamps).
 */
export function dragTo(gesture: Gesture, frame: number, options: {
  media: readonly Media[]; track?: string; overwrite?: boolean; ripple?: boolean; vertical?: boolean; candidates?: readonly number[]; scale?: number;
}): Dragged {
  const { timeline, locations, action, ids, linked } = gesture;
  const here = locations.get(gesture.clip)!;
  const moving = ids.map(id => locations.get(id)!);
  const upright = options.vertical === true && action === 'move' && !options.ripple;
  let delta = upright ? 0 : Math.round(frame - gesture.origin);
  let point: number | null = null;
  const together = new Set(ids);
  const anchors = action === 'move' || action === 'slide' ? moving.flatMap(({ clip }) => [clip.start, clip.start + clip.length])
    : action === 'trim_in' ? [here.clip.start] : action === 'trim_out' ? [here.clip.start + here.clip.length]
      : typeof action === 'object' ? [locations.get(action.right)!.clip.start] : [];
  if (options.candidates && anchors.length && !upright) ({ delta, point } = snap(delta, anchors, options.candidates, options.scale ?? 1));
  const requested = delta;
  const clamp = (low: number, high: number) => { delta = Math.max(low, Math.min(high, delta)); };
  if (action === 'move') clamp(-Math.min(...moving.map(({ clip }) => clip.start)), Infinity);
  else if (action === 'trim_in' || action === 'trim_out') {
    for (const located of moving) {
      const { clip } = located;
      const available = handles(clip, timeline, options.media);
      const before = clip.reverse ? available.after : available.before;
      const after = clip.reverse ? available.before : available.after;
      // Without ripple an edge stops at the clip beside it; with ripple the rest moves along.
      const free = gesture.rippleTrim ? { before: Infinity, after: Infinity } : room(located, together);
      if (action === 'trim_in') clamp(Math.max(-before, -free.before), clip.length - 1);
      else clamp(1 - clip.length, Math.min(after, free.after));
    }
  } else if (action === 'slip') {
    for (const { clip } of moving) { const available = handles(clip, timeline, options.media); clamp(-available.before, available.after); }
  } else if (action === 'slide') clamp(-Math.min(...moving.map(({ clip }) => clip.start)), Infinity);
  else {
    const left = locations.get(action.left)!.clip;
    const right = locations.get(action.right)!.clip;
    const a = handles(left, timeline, options.media);
    const b = handles(right, timeline, options.media);
    clamp(Math.max(1 - left.length, -(right.reverse ? b.after : b.before)), Math.min(right.length - 1, left.reverse ? a.before : a.after));
  }
  if (delta !== requested) point = null;
  if (delta === 0) delta = 0;
  const preview: Preview[] = moving.map(({ clip, track }) => ({ id: clip.id, track: track.id, start: clip.start, length: clip.length }));
  let command: Edit;
  const common = { timeline: timeline.id, delta, linked };
  if (action === 'move') {
    const destination = options.ripple ? undefined : timeline.tracks.find(track => track.id === options.track && track.kind === here.track.kind);
    const peers = timeline.tracks.filter(track => track.kind === here.track.kind);
    const shift = destination ? peers.findIndex(track => track.id === destination.id) - peers.findIndex(track => track.id === here.track.id) : 0;
    for (const item of preview) {
      item.start += delta;
      const original = locations.get(item.id)!;
      if (original.track.kind === here.track.kind) item.track = peers[peers.findIndex(track => track.id === original.track.id) + shift]?.id ?? item.track;
    }
    command = options.ripple ? { edit: 'insert_move', timeline: timeline.id, clips: ids, at: Math.max(0, Math.min(...moving.map(({ clip }) => clip.start)) + delta), linked }
      : { edit: 'move', ...common, clips: ids, ...(destination && destination.id !== here.track.id ? { to_track: destination.id } : {}), overwrite: options.overwrite ?? true };
  } else if (action === 'trim_in' || action === 'trim_out') {
    for (const item of preview) {
      if (action === 'trim_in') { if (!gesture.rippleTrim) item.start += delta; item.length -= delta; }
      else item.length += delta;
    }
    command = { edit: 'trim', ...common, clip: gesture.clip, edge: action === 'trim_in' ? 'in' : 'out', ripple: gesture.rippleTrim };
  } else if (action === 'slip' || action === 'slide') {
    if (action === 'slide') for (const item of preview) item.start += delta;
    command = { edit: action, ...common, clip: gesture.clip };
  } else {
    const left = locations.get(action.left)!;
    const right = locations.get(action.right)!;
    preview.splice(0, preview.length,
      { id: left.clip.id, track: left.track.id, start: left.clip.start, length: left.clip.length + delta },
      { id: right.clip.id, track: right.track.id, start: right.clip.start + delta, length: right.clip.length - delta });
    command = { edit: 'roll', ...common, left: action.left, right: action.right };
  }
  const changedTrack = command.edit === 'move' && command.to_track !== undefined;
  const covered = command.edit === 'move' && command.overwrite ? overwritten(timeline, preview, together) : [];
  return { gesture, delta, point, clamped: delta !== requested, preview, covered, command: delta !== 0 || changedTrack ? command : null };
}

/** The parts of the clips that stay which the moved clips would cover, track by track. */
function overwritten(timeline: Timeline, preview: readonly Preview[], moving: ReadonlySet<string>): Preview[] {
  const covered: Preview[] = [];
  for (const item of preview) {
    const track = timeline.tracks.find(each => each.id === item.track);
    if (!track || track.kind === 'subtitle') continue;
    const end = item.start + item.length;
    for (const clip of track.clips) {
      if (moving.has(clip.id) || clip.start >= end || clip.start + clip.length <= item.start) continue;
      const start = Math.max(clip.start, item.start);
      covered.push({ id: clip.id, track: track.id, start, length: Math.min(clip.start + clip.length, end) - start });
    }
  }
  return covered;
}

export function finishDrag(dragged: Dragged): Transaction | null {
  return dragged.command ? { revision: dragged.gesture.revision, ops: [dragged.command] } : null;
}
