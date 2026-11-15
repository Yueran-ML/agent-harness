import type { Clip, Media, Timeline } from '../engine';
import type { Change, Edit, Transaction } from './commands';
import { clipLocations, movableClips, editingPeers, type Located } from './selection';
import { snap } from './snap';
import { retimeCourse, type Course } from '../retime';

export type Tool = 'select' | 'trim' | 'blade';
export type Hit = 'move' | 'trim_in' | 'trim_out' | 'slip' | 'slide' | 'blade' | { kind: 'roll'; left: string; right: string };
export interface Preview { id: string; track: string; start: number; length: number }
export interface Gesture {
  /** `ids`: the clips that the gesture moves or trims, none of them on a locked track (see `movableClips`). */
  timeline: Timeline; revision: number; ids: string[]; clip: string; action: Exclude<Hit, 'blade'>;
  origin: number; linked: boolean; rippleTrim: boolean; locations: ReadonlyMap<string, Located>;
  /** Clips linked to them on locked tracks: they stay where they are, and are not named to the engine. */
  lockedPartners: string[];
}
export interface Dragged {
  gesture: Gesture; delta: number; point: number | null; clamped: boolean; preview: Preview[]; command: Edit | null;
  /** What a move would overwrite of the clips that stay: parts of them, on their tracks. */
  covered: Preview[];
  /** Edits done after the command, in the same transaction: the gaps that a move leaves on Track 1 of the Cut page, closed. */
  then?: Edit[];
  /** A move with Alt held: copies put where the clips would go, the clips staying (instead of the command). */
  duplicate?: Change[];
  /** Why a move with Alt held copies nothing: a copy would go on a locked track, or where there is no track for it. */
  refused?: 'copy_locked' | 'copy_no_track' | 'move_no_track';
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
  const { movable: ids, lockedPartners } = movableClips(timeline, action === 'move' && selection.includes(id) ? [id, ...selection] : [id], linked);
  return { timeline, revision, ids, clip: id, action, origin, linked, rippleTrim, locations, lockedPartners };
}

/** The most frames from 0 on that `fits` holds for, where it holds for every count below one that it holds for; `Infinity` where it never fails. */
function most(fits: (frames: number) => boolean): number {
  if (!fits(1)) return 0;
  let low = 1;
  let high = 2;
  while (fits(high)) {
    low = high;
    high *= 2;
    if (high > 2 ** 40) return Infinity;
  }
  while (high - low > 1) {
    const middle = low + Math.floor((high - low) / 2);
    if (fits(middle)) low = middle;
    else high = middle;
  }
  return low;
}

/**
 * How many frames the head of a clip with speed points may be moved out, and its tail, before it
 * would show what is not in its media: its first segment goes on before the clip and its last
 * after it (`Retime::from_frame`, `Retime::with_length`), and what it then shows, from the lowest
 * place of the path to where the highest ends (`Course::shown`, `Course::reach`), lies within the
 * media. The engine counts the same, and rounds where the clip then begins to a tick: where a
 * frame lasts no whole number of ticks this may be a frame more than the engine gives, and the
 * engine's answer is what is drawn in the end.
 */
function retimedRoom(course: Course, begins: { ticks: number; scale: number }, duration: { ticks: number; scale: number }, timeline: Timeline): { before: number; after: number } {
  const fits = (before: number, after: number) => {
    const lowest = BigInt(begins.ticks) + course.shown(before, after, timeline.rate, begins.scale)[0];
    const end = BigInt(begins.ticks) + course.reach(before, after, timeline.rate, begins.scale);
    return lowest >= 0n && end * BigInt(duration.scale) <= BigInt(duration.ticks) * BigInt(begins.scale);
  };
  return { before: most(frames => fits(frames, 0)), after: most(frames => fits(0, frames)) };
}

/**
 * The frames of its media that a clip has beyond its ends, at its speed: `before` those before the
 * place where it begins in its media, `after` those after what it shows (a clip that plays
 * backward shows them at its other end, which `dragTo` turns round).
 *
 * A clip with speed points has no one speed to count them at. For a trim or a roll they are the
 * frames that its head and its tail may be moved out by (`retimedRoom`), in the order of the
 * timeline. For a slip (`slipped`), which begins the clip as many frames of its source later at
 * the speed of the source, they are the frames of the media before the lowest place of its path
 * and after the highest.
 */
function handles(clip: Clip, timeline: Timeline, media: readonly Media[], slipped = false): { before: number; after: number } {
  if (clip.source.kind !== 'media') return { before: Infinity, after: Infinity };
  const source = clip.source;
  const item = media.find(item => item.id === source.media);
  if (item?.facts.video?.still) return { before: Infinity, after: Infinity };
  if (!item) return { before: 0, after: 0 };
  const course = retimeCourse(clip);
  if (course && !slipped) return retimedRoom(course, source.in, item.facts.duration, timeline);
  if (course) {
    const frame = timeline.rate.den / timeline.rate.num;
    const begins = source.in.ticks / source.in.scale;
    const shown = Number(course.span(timeline.rate, source.in.scale)) / source.in.scale;
    const ends = item.facts.duration.ticks / item.facts.duration.scale;
    return { before: Math.floor(begins / frame + 1e-8), after: Math.floor((ends - begins - shown) / frame + 1e-8) };
  }
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
 *
 * `magnetic` is Track 1 of the Cut page (see `primaryTrack`): a clip of it that is moved leaves no
 * gap there, as on DaVinci Resolve's Cut page (Reference Manual 20, pp. 584–588). Dropped with the
 * pointer on an edit point between two of its clips, it is put in there and the rest move aside
 * (a shuffle insert, "swapping clips"); dropped elsewhere it overwrites what it covers, and the gap
 * that it leaves on Track 1 is closed, the rest of the timeline moving back; clips of other tracks
 * across that gap stay where they are (`close_gap` with `straddling: 'stay'`).
 */
export function dragTo(gesture: Gesture, frame: number, options: {
  media: readonly Media[]; track?: string; overwrite?: boolean; ripple?: boolean; vertical?: boolean; candidates?: readonly number[]; scale?: number;
  magnetic?: { track: string; clips: ReadonlySet<string> } | null;
  /** Alt held: the clips are copied there, and stay where they are (Reference Manual 20, p. 811). */
  duplicate?: boolean;
}): Dragged {
  const { timeline, locations, action, ids, linked } = gesture;
  const here = locations.get(gesture.clip)!;
  const copying = options.duplicate === true && action === 'move' && !options.ripple;
  // A move leaves the clips linked to it on locked tracks where they are, and the engine moves the
  // rest. A copy with Alt copies the whole group, so a copy of one of those that would go on a locked
  // track refuses it (`copy_locked`), as one of any clip would.
  const moving = (copying ? [...ids, ...gesture.lockedPartners] : ids).map(id => locations.get(id)!);
  const magnetic = !copying && options.magnetic && action === 'move' && !options.ripple && !options.vertical && options.magnetic.clips.has(gesture.clip) ? options.magnetic : null;
  // The pointer on an edit point of Track 1 between two clips that stay: the clips go in there.
  if (magnetic && (options.track === undefined || options.track === magnetic.track)) {
    const cut = swapPoint(timeline, magnetic.track, new Set(ids), frame, options.scale ?? 1);
    if (cut !== null) {
      const first = Math.min(...moving.map(({ clip }) => clip.start));
      const preview = moving.map(({ clip, track }) => ({ id: clip.id, track: track.id, start: clip.start - first + cut, length: clip.length }));
      return { gesture, delta: cut - first, point: cut, clamped: false, preview, covered: [],
        command: { edit: 'insert_move', timeline: timeline.id, clips: ids, at: cut, linked, ...(timeline.track_groups?.length ? { grouped: true } : {}) } };
    }
  }
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
    for (const { clip } of moving) { const available = handles(clip, timeline, options.media, true); clamp(-available.before, available.after); }
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
  // Whether a clip moved to another track would go past the last track of its kind.
  let trackless = false;
  let command: Edit;
  const common = { timeline: timeline.id, delta, linked };
  if (action === 'move') {
    const destination = options.ripple ? undefined : timeline.tracks.find(track => track.id === options.track && track.kind === here.track.kind);
    const peers = (kind: string) => timeline.tracks.filter(track => track.kind === kind);
    const place = (track: Timeline['tracks'][number]) => peers(track.kind).findIndex(each => each.id === track.id);
    const shift = destination ? place(destination) - place(here.track) : 0;
    for (const item of preview) {
      item.start += delta;
      if (shift === 0) continue;
      // Every clip goes as many tracks along its own kind: the sound linked to a picture moved
      // from V1 to V2 goes from A1 to A2 (Linked Move Across Tracks, Reference Manual 20, p. 769).
      const original = locations.get(item.id)!;
      const to = peers(original.track.kind)[place(original.track) + shift];
      if (to) item.track = to.id; else trackless = true;
    }
    command = options.ripple ? { edit: 'insert_move', timeline: timeline.id, clips: ids, at: Math.max(0, Math.min(...moving.map(({ clip }) => clip.start)) + delta), linked, ...(timeline.track_groups?.length ? { grouped: true } : {}) }
      : { edit: 'move', ...common, ...(timeline.track_groups?.length ? { grouped: true } : {}), clips: ids, ...(destination && destination.id !== here.track.id ? { to_track: destination.id } : {}), overwrite: options.overwrite ?? true };
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
  if (copying) {
    // The copies overwrite what they cover, the clips they are copied from too.
    const copied = delta !== 0 || changedTrack ? trackless ? 'copy_no_track' as const : duplicated(timeline, locations, preview) : null;
    const ops = Array.isArray(copied) ? copied : null;
    return { gesture, delta, point, clamped: delta !== requested, preview, covered: ops ? overwritten(timeline, preview, new Set()) : [], command: null,
      ...(ops ? { duplicate: ops } : typeof copied === 'string' ? { refused: copied } : {}) };
  }
  // A clip, or a clip linked to it, with no track of its kind to go to: the engine makes no track
  // and refuses the move (`no_track`), so it is not sent, and the studio says why.
  if (changedTrack && trackless) return { gesture, delta, point, clamped: delta !== requested, preview, covered: [], command: null, refused: 'move_no_track' };
  const covered = command.edit === 'move' && command.overwrite ? overwritten(timeline, preview, together) : [];
  let done = delta !== 0 || changedTrack ? command : null;
  const gaps = magnetic && done ? closeLeft(timeline, magnetic.track, moving, preview) : [];
  // Clips moved later into empty room, with nothing after them, come back where they were as the
  // gap closes: that is no change, and no step to undo.
  if (gaps.length === 1 && !changedTrack && covered.length === 0 && gaps[0].to - gaps[0].at === delta
    && timeline.tracks.every(track => track.locked || (track.kind === 'subtitle' ? track.cues.every(cue => cue.start < gaps[0].to)
      : track.clips.every(clip => together.has(clip.id) || clip.start < gaps[0].to)))) {
    done = null;
    gaps.length = 0;
  }
  // Only Track 1 is closed up: clips of the tracks above or of sound that lie across the gap, as a
  // bed of music or the clip just moved up off it does, stay where they are, and those after it move
  // back with Track 1, superimposed clips "kept in sync" (Reference Manual 20, pp. 586, 588).
  const then: Edit[] = gaps.map(({ at }) => ({ edit: 'close_gap', timeline: timeline.id, track: magnetic!.track, at, straddling: 'stay' }));
  return { gesture, delta, point, clamped: delta !== requested, preview, covered, command: done, ...(then.length > 0 ? { then } : {}) };
}

/** How near, in pixels, the pointer is to an edit point of Track 1 for a clip to be put in there. */
const SWAP_PX = 8;

/**
 * The edit point of `track` within reach of `frame` where two clips meet that do not move (the
 * nearest; of two as near, the earlier), else null.
 */
function swapPoint(timeline: Timeline, track: string, moving: ReadonlySet<string>, frame: number, scale: number): number | null {
  const lane = timeline.tracks.find(each => each.id === track);
  if (!lane || lane.kind === 'subtitle') return null;
  let best: number | null = null;
  for (let at = 1; at < lane.clips.length; at++) {
    const left = lane.clips[at - 1];
    const right = lane.clips[at];
    if (left.start + left.length !== right.start || moving.has(left.id) || moving.has(right.id)) continue;
    const distance = Math.abs(right.start - frame) * scale;
    if (distance <= SWAP_PX && (best === null || distance < Math.abs(best - frame) * scale)) best = right.start;
  }
  return best;
}

/** The stretches that `spans` cover, in order, those that meet or overlap made one. */
function union(spans: readonly [number, number][]): [number, number][] {
  const all: [number, number][] = [];
  for (const [from, to] of [...spans].sort((a, b) => a[0] - b[0])) {
    const last = all.at(-1);
    if (last && from <= last[1]) last[1] = Math.max(last[1], to);
    else all.push([from, to]);
  }
  return all;
}

/**
 * The gaps that clips moved off their places leave on `track`, to be closed: each empty stretch
 * (after the move) that a place they left lies in, from `at` to `to`, the latest first, so that
 * closing one does not move the others. A stretch with nothing after it is no gap to close.
 */
function closeLeft(timeline: Timeline, track: string, moving: readonly Located[], preview: readonly Preview[]): { at: number; to: number }[] {
  const lane = timeline.tracks.find(each => each.id === track);
  if (!lane || lane.kind === 'subtitle') return [];
  const ids = new Set(moving.map(({ clip }) => clip.id));
  const placed = preview.filter(item => item.track === track).map(item => [item.start, item.start + item.length] as [number, number]);
  const taken = union([...lane.clips.filter(clip => !ids.has(clip.id)).map(clip => [clip.start, clip.start + clip.length] as [number, number]), ...placed]);
  const starts = new Map<number, number>();
  for (const { clip, track: from } of moving) {
    if (from.id !== track) continue;
    // The frames of its old place that nothing covers after the move, a stretch at a time.
    for (let at = clip.start; at < clip.start + clip.length;) {
      const over = taken.find(([a, b]) => a <= at && at < b);
      if (over) { at = over[1]; continue; }
      const before = taken.filter(([, b]) => b <= at).reduce((most, [, b]) => Math.max(most, b), 0);
      const after = taken.find(([a]) => a > at);
      if (!after) break;
      starts.set(before, after[0]);
      at = after[0];
    }
  }
  return [...starts].sort((a, b) => b[0] - a[0]).map(([at, to]) => ({ at, to }));
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

/**
 * The edits that copy clips to where `preview` puts them, as DaVinci Resolve duplicates clips that
 * are Option-dragged (Reference Manual 20, p. 811), in one transaction: what lies where the copies go
 * is cleared as an overwrite clears it (see `clearing`), and each copy is put there with all the
 * settings of its clip (speed, reverse, transform, gain, pan, fades, whether it is enabled); copies
 * of clips linked together are linked together, in a new group. Transitions are not copied. Where a
 * copy would go on a track that is locked (or one of subtitles), why nothing is copied.
 */
function duplicated(timeline: Timeline, locations: ReadonlyMap<string, Located>, preview: readonly Preview[]): Change[] | 'copy_locked' {
  if (preview.some(item => editingPeers(timeline, locations.get(item.id)!.track.id).length > 0)) {
    for (const item of preview) if (timeline.tracks.find(track => track.id === item.track)?.locked) return 'copy_locked';
    return [{ edit: 'place_copies', timeline: timeline.id, ...(timeline.track_groups?.length ? { grouped: true } : {}), copies: preview.map(item => ({ clip: item.id, track: item.track, start: item.start })) }];
  }
  const byTrack = new Map<string, [number, number][]>();
  for (const item of preview) byTrack.set(item.track, [...byTrack.get(item.track) ?? [], [item.start, item.start + item.length]]);
  const ops: Change[] = [];
  for (const [track, spans] of byTrack) {
    const lane = timeline.tracks.find(each => each.id === track);
    if (!lane || lane.kind === 'subtitle' || lane.locked) return 'copy_locked';
    ops.push(...clearing(timeline.id, lane.clips, spans, track));
  }
  // A group of which two or more clips are copied is copied as a new group; a copy alone is in none.
  const groups = new Map<string, number>();
  for (const item of preview) { const link = locations.get(item.id)!.clip.link; if (link) groups.set(link, (groups.get(link) ?? 0) + 1); }
  const names = new Map([...groups].filter(([, count]) => count > 1).map(([link], at) => [link, `$copy${at + 1}`]));
  for (const item of preview) {
    const { clip } = locations.get(item.id)!;
    ops.push({ op: 'insert_clip', timeline: timeline.id, track: item.track, clip: { ...clip, id: '', start: item.start, link: clip.link ? names.get(clip.link) ?? null : null } });
  }
  return ops;
}

/**
 * The edits that empty the frames `spans` of a track as an overwrite does, the rest staying: a clip
 * inside is lifted, one across an end is trimmed back to it, one across both is split at the end
 * and its first part trimmed to the start. The spans are done from the last to the first, so that
 * each edit names a clip by the id it has then (a split keeps the id on its first part).
 */
export function clearing(timeline: string, clips: readonly Clip[], spans: readonly [number, number][], track: string): Edit[] {
  const left = clips.map(clip => ({ id: clip.id, start: clip.start, end: clip.start + clip.length, gone: false }));
  const lifted: string[] = [];
  const edits: Edit[] = [];
  for (const [from, to] of union(spans).reverse()) {
    for (const clip of left) {
      if (clip.gone || clip.end <= from || clip.start >= to) continue;
      if (clip.start >= from && clip.end <= to) { lifted.push(clip.id); clip.gone = true; }
      else if (clip.start < from && clip.end > to) {
        edits.push({ edit: 'blade', timeline, at: to, tracks: [track], linked: false });
        edits.push({ edit: 'trim', timeline, clip: clip.id, edge: 'out', delta: from - to, ripple: false, linked: false });
        clip.end = from;
      } else if (clip.start < from) { edits.push({ edit: 'trim', timeline, clip: clip.id, edge: 'out', delta: from - clip.end, ripple: false, linked: false }); clip.end = from; }
      else { edits.push({ edit: 'trim', timeline, clip: clip.id, edge: 'in', delta: to - clip.start, ripple: false, linked: false }); clip.start = to; }
    }
  }
  return lifted.length > 0 ? [{ edit: 'lift', timeline, clips: lifted, linked: false }, ...edits] : edits;
}

export function finishDrag(dragged: Dragged): Transaction | null {
  if (dragged.duplicate) return { revision: dragged.gesture.revision, ops: dragged.duplicate };
  return dragged.command ? { revision: dragged.gesture.revision, ops: [dragged.command, ...(dragged.then ?? [])] } : null;
}
