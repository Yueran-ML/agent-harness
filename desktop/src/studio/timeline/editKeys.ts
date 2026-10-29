import type { Key } from '../../app/keys';
import type { Media, Monitor, Op, Rate, Time, Timeline } from '../engine';
import { duration, snapPoints } from './geometry';
import type { Change, Edit, Material, Targets, Transaction } from './commands';
import type { Tool } from './interact';
import { clipLocations, linkedClips } from './selection';
import type { Marks } from './snap';
import { materialLength } from './drop';

export type EditAction =
  | { kind: 'tool'; tool: Tool }
  | { kind: 'snap' | 'linked' | 'fit' | 'cancel' | 'speed' | 'enabled' | 'lock' | 'transition' | 'blade' }
  | { kind: 'history'; method: 'history.undo' | 'history.redo' }
  | { kind: 'mark'; edge: 'in' | 'out' | 'clear' }
  | { kind: 'place'; edit: 'insert' | 'overwrite' | 'replace' | 'place_on_top' | 'append' }
  | { kind: 'delete'; ripple: boolean }
  | { kind: 'cut'; direction: -1 | 1 }
  | { kind: 'zoom'; direction: -1 | 1 };

/** Classify first so modified shortcuts cannot fall through to their unmodified tool or fit key. */
export function editKey(key: Key): EditAction | null {
  if (key.composing || key.repeat && !['ArrowUp', 'ArrowDown'].includes(key.key)) return null;
  const value = key.key.length === 1 ? key.key.toLowerCase() : key.key;
  const control = key.ctrl || key.meta;
  if (control) {
    if (value === 'l' && key.alt && !key.shift) return { kind: 'lock' };
    if (key.alt) return null;
    // Ctrl with plus or minus zooms the timeline, as in DaVinci Resolve; plus is often typed with Shift.
    if (value === '=' || value === '+') return { kind: 'zoom', direction: 1 };
    if (value === '-' && !key.shift) return { kind: 'zoom', direction: -1 };
    if (value === 'z') return { kind: 'history', method: key.shift ? 'history.redo' : 'history.undo' };
    if (value === 'l' && key.shift) return { kind: 'linked' };
    if (key.shift) return null;
    if (value === 'b' || value === '\\') return { kind: 'blade' };
    if (value === 't') return { kind: 'transition' };
    return null;
  }
  if (key.alt) return value === 'x' && !key.shift ? { kind: 'mark', edge: 'clear' } : null;
  if (key.shift) {
    if (value === 'z') return { kind: 'fit' };
    if (value === 'F12') return { kind: 'place', edit: 'append' };
    if (value === 'Delete' || value === 'Backspace') return { kind: 'delete', ripple: true };
    return null;
  }
  switch (value) {
    case 'a': return { kind: 'tool', tool: 'select' };
    case 't': return { kind: 'tool', tool: 'trim' };
    case 'b': return { kind: 'tool', tool: 'blade' };
    case 'n': return { kind: 'snap' };
    case 'i': return { kind: 'mark', edge: 'in' };
    case 'o': return { kind: 'mark', edge: 'out' };
    case 'r': return { kind: 'speed' };
    case 'd': return { kind: 'enabled' };
    case 'Escape': return { kind: 'cancel' };
    // As DaVinci Resolve has them: Backspace (Delete on a Mac) lifts and leaves a gap; Delete
    // (Forward Delete on a Mac), like Shift with either, ripple deletes.
    case 'Backspace': return { kind: 'delete', ripple: false };
    case 'Delete': return { kind: 'delete', ripple: true };
    case 'F9': return { kind: 'place', edit: 'insert' };
    case 'F10': return { kind: 'place', edit: 'overwrite' };
    case 'F11': return { kind: 'place', edit: 'replace' };
    case 'F12': return { kind: 'place', edit: 'place_on_top' };
    case 'ArrowUp': return { kind: 'cut', direction: -1 };
    case 'ArrowDown': return { kind: 'cut', direction: 1 };
    default: return null;
  }
}

export interface EditContext {
  timeline: Timeline | null; revision: number; selected: readonly string[]; linked: boolean;
  targets: Targets; marks: Marks; frame: number; focus: Monitor;
  source: { media: Media; frame: number; rate: Rate } | null;
}
export type EditResult =
  | { kind: 'transaction'; transaction: Transaction }
  | { kind: 'history'; method: 'history.undo' | 'history.redo' }
  | { kind: 'seek'; monitor: Monitor; frame: number }
  | { kind: 'marks'; marks: Marks }
  | { kind: 'speed'; clips: string[] }
  | { kind: 'tool'; tool: Tool }
  | { kind: 'snap' | 'linked' | 'fit' | 'cancel' }
  | { kind: 'zoom'; direction: -1 | 1 };

/** Integer rational conversion prevents a frame boundary from rounding one source tick too early. */
export function sourceTime(frame: number, rate: Rate, scale: number): Time {
  return { ticks: Number(BigInt(Math.max(0, Math.floor(frame))) * BigInt(rate.den) * BigInt(scale) / BigInt(rate.num)), scale };
}
/** The source ticks that `frames` of a timeline at `rate` take, at speed 1, rounded up to a whole tick. */
function sourceSpan(frames: number, rate: Rate, scale: number): number {
  const product = BigInt(Math.max(0, frames)) * BigInt(rate.den) * BigInt(scale);
  const num = BigInt(rate.num);
  return Number((product + num - 1n) / num);
}

/**
 * Where an edit puts the source and which part of it, by the rules of three-point editing of
 * DaVinci Resolve (Reference Manual 20, "The Rules of Three-Point Editing"):
 *
 * - Without an In on the source, its first frame is the In; without an Out, its last frame. The
 *   engine takes the marks of the media and these defaults itself, so they are not sent.
 * - Timeline In and Out: they decide the length, and the source In goes to the timeline In. With
 *   only an Out on the source, the edit is backtimed: the source Out goes to the timeline Out. With
 *   all four marks, those of the timeline win and the source Out is passed over.
 * - Only a timeline Out: the edit is backtimed, so that the source Out goes to the timeline Out.
 *   Where the source is longer than the time before that Out, it begins at the start of the
 *   timeline, and only its end that fits is used.
 * - Neither: the playhead of the timeline is the timeline In.
 *
 * A replace edit lines up the frame of the source monitor with the playhead of the timeline.
 */
export function threePoint(timeline: Timeline, source: NonNullable<EditContext['source']>, marks: Marks, frame: number, replace = false): { at: number; material: Material } {
  const { media } = source;
  const scale = media.facts.duration.scale;
  const material: Material = { kind: 'media', media: media.id };
  const still = media.facts.video?.still === true;
  if (replace) {
    material.from = sourceTime(source.frame, source.rate, scale);
    return { at: marks.in ?? frame, material };
  }
  const end = media.mark_out?.ticks ?? media.facts.duration.ticks;
  if (marks.in !== null && marks.out !== null && marks.out > marks.in) {
    material.length = marks.out - marks.in;
    if (!media.mark_in && media.mark_out && !still) material.from = { ticks: Math.max(0, end - sourceSpan(material.length, timeline.rate, scale)), scale };
    return { at: marks.in, material };
  }
  if (marks.in === null && marks.out !== null) {
    const length = materialLength(media, timeline.rate);
    if (length <= marks.out) return { at: marks.out - length, material };
    material.length = marks.out;
    if (!still) material.from = { ticks: Math.max(0, end - sourceSpan(marks.out, timeline.rate, scale)), scale };
    return { at: 0, material };
  }
  return { at: marks.in ?? frame, material };
}

const compare = (a: Time, b: Time) => BigInt(a.ticks) * BigInt(b.scale) - BigInt(b.ticks) * BigInt(a.scale);

export function editCommand(action: EditAction | null, context: EditContext): EditResult | null {
  if (!action) return null;
  const { timeline, source, revision, linked, marks, frame, targets } = context;
  const transaction = (ops: Change[]): EditResult | null => ops.length ? { kind: 'transaction', transaction: { revision, ops } } : null;
  if (action.kind === 'history' || action.kind === 'tool' || action.kind === 'zoom' || ['snap', 'linked', 'fit', 'cancel'].includes(action.kind)) return action as EditResult;
  if (action.kind === 'mark') {
    if (context.focus === 'source') {
      if (!source) return null;
      const { media } = source;
      const op: Extract<Op, { op: 'set_media' }> = { op: 'set_media', id: media.id };
      if (action.edge === 'clear') { op.mark_in = null; op.mark_out = null; }
      else {
        const time = sourceTime(source.frame + (action.edge === 'out' ? 1 : 0), source.rate, media.facts.duration.scale);
        time.ticks = Math.min(time.ticks, media.facts.duration.ticks);
        if (action.edge === 'in') { op.mark_in = time; if (media.mark_out && compare(time, media.mark_out) >= 0n) op.mark_out = null; }
        else { op.mark_out = time; if (media.mark_in && compare(time, media.mark_in) <= 0n) op.mark_in = null; }
      }
      // A mark set where it is already is no change; the engine refuses a step that changes nothing.
      const same = (a: Time | null | undefined, b: Time | null) => (a ?? null) === null ? b === null : b !== null && compare(a!, b) === 0n;
      if (op.mark_in !== undefined && same(op.mark_in, media.mark_in)) delete op.mark_in;
      if (op.mark_out !== undefined && same(op.mark_out, media.mark_out)) delete op.mark_out;
      return op.mark_in === undefined && op.mark_out === undefined ? null : transaction([op]);
    }
    if (!timeline) return null;
    const next = { ...marks };
    if (action.edge === 'clear') { next.in = null; next.out = null; }
    else if (action.edge === 'in') { next.in = frame; if (next.out !== null && next.out <= frame) next.out = null; }
    else { next.out = frame + 1; if (next.in !== null && next.in >= next.out) next.in = null; }
    return { kind: 'marks', marks: next };
  }
  if (!timeline) return null;
  const locations = clipLocations(timeline);
  const selected = linkedClips(timeline, context.selected, linked);
  const chosenTracks = [...new Set(selected.map(id => locations.get(id)!.track.id))];
  const activeTracks = Object.values(targets).filter((id): id is string => typeof id === 'string');
  if (action.kind === 'place') {
    if (!source) return null;
    if (action.edit === 'append') return transaction([{ edit: 'append', timeline: timeline.id, source: { kind: 'media', media: source.media.id }, ...targets }]);
    const placed = threePoint(timeline, source, marks, frame, action.edit === 'replace');
    const command: Edit = action.edit === 'place_on_top' ? { edit: action.edit, timeline: timeline.id, source: placed.material, at: placed.at }
      : { edit: action.edit, timeline: timeline.id, source: placed.material, at: placed.at, ...targets };
    return transaction([command]);
  }
  if (action.kind === 'delete') {
    if (action.ripple && marks.in !== null && marks.out !== null && marks.out > marks.in) return transaction([{ edit: 'ripple_delete_range', timeline: timeline.id, from: marks.in, to: marks.out }]);
    return selected.length ? transaction([{ edit: action.ripple ? 'ripple_delete' : 'lift', timeline: timeline.id, clips: selected, linked }]) : null;
  }
  if (action.kind === 'blade') {
    const tracks = chosenTracks.length ? chosenTracks : activeTracks;
    return tracks.length ? transaction([{ edit: 'blade', timeline: timeline.id, at: frame, tracks, linked }]) : null;
  }
  if (action.kind === 'cut') {
    const points = snapPoints(timeline);
    const next = action.direction < 0 ? points.reverse().find(point => point < frame) ?? 0 : points.find(point => point > frame) ?? duration(timeline);
    return { kind: 'seek', monitor: 'timeline', frame: Math.max(0, Math.min(duration(timeline) - 1, next)) };
  }
  if (action.kind === 'speed') return selected.length ? { kind: 'speed', clips: selected } : null;
  if (action.kind === 'enabled') {
    const enabled = selected.every(id => !locations.get(id)!.clip.enabled);
    return transaction(selected.map(id => ({ op: 'set_clip', timeline: timeline.id, id, enabled })));
  }
  if (action.kind === 'lock') {
    const ids = chosenTracks.length ? chosenTracks : activeTracks;
    const tracks = timeline.tracks.filter(track => ids.includes(track.id));
    const locked = !tracks.every(track => track.locked);
    return transaction(tracks.map(track => ({ op: 'set_track', timeline: timeline.id, id: track.id, locked })));
  }
  if (action.kind === 'transition') {
    const edits: Edit[] = [];
    for (const track of timeline.tracks) if (track.kind !== 'subtitle') for (let i = 1; i < track.clips.length; i++) {
      const left = track.clips[i - 1];
      const right = track.clips[i];
      if (left.start + left.length !== right.start || track.transitions.some(t => t.left === left.id && t.right === right.id)) continue;
      if (selected.length ? !selected.includes(left.id) && !selected.includes(right.id) : right.start !== frame || !activeTracks.includes(track.id)) continue;
      edits.push({ edit: 'add_transition', timeline: timeline.id, left: left.id, right: right.id, kind: track.kind === 'audio' ? 'crossfade' : 'dissolve', length: Math.max(1, Math.round(timeline.rate.num / timeline.rate.den)), align: 'center' });
    }
    return transaction(edits);
  }
  return null;
}
