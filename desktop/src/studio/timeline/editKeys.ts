import type { Key } from '../../app/keys';
import type { Media, Monitor, Op, Rate, Time, Timeline } from '../engine';
import { duration, snapPoints } from './geometry';
import type { Change, Edit, Material, Targets, Transaction } from './commands';
import type { Tool } from './interact';
import { clipLocations, linkedClips } from './selection';
import type { Marks } from './snap';

export type EditAction =
  | { kind: 'tool'; tool: Tool }
  | { kind: 'snap' | 'linked' | 'fit' | 'cancel' | 'speed' | 'enabled' | 'lock' | 'transition' | 'blade' }
  | { kind: 'history'; method: 'history.undo' | 'history.redo' }
  | { kind: 'mark'; edge: 'in' | 'out' | 'clear' }
  | { kind: 'place'; edit: 'insert' | 'overwrite' | 'replace' | 'place_on_top' | 'append' }
  | { kind: 'delete'; ripple: boolean }
  | { kind: 'cut'; direction: -1 | 1 };

/** Classify first so modified shortcuts cannot fall through to their unmodified tool or fit key. */
export function editKey(key: Key): EditAction | null {
  if (key.composing || key.repeat && !['ArrowUp', 'ArrowDown'].includes(key.key)) return null;
  const value = key.key.length === 1 ? key.key.toLowerCase() : key.key;
  const control = key.ctrl || key.meta;
  if (control) {
    if (value === 'l' && key.alt && !key.shift) return { kind: 'lock' };
    if (key.alt) return null;
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
    case 'Delete': case 'Backspace': return { kind: 'delete', ripple: false };
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
  | { kind: 'snap' | 'linked' | 'fit' | 'cancel' };

/** Integer rational conversion prevents a frame boundary from rounding one source tick too early. */
export function sourceTime(frame: number, rate: Rate, scale: number): Time {
  return { ticks: Number(BigInt(Math.max(0, Math.floor(frame))) * BigInt(rate.den) * BigInt(scale) / BigInt(rate.num)), scale };
}
const compare = (a: Time, b: Time) => BigInt(a.ticks) * BigInt(b.scale) - BigInt(b.ticks) * BigInt(a.scale);

export function editCommand(action: EditAction | null, context: EditContext): EditResult | null {
  if (!action) return null;
  const { timeline, source, revision, linked, marks, frame, targets } = context;
  const transaction = (ops: Change[]): EditResult | null => ops.length ? { kind: 'transaction', transaction: { revision, ops } } : null;
  if (action.kind === 'history' || action.kind === 'tool' || ['snap', 'linked', 'fit', 'cancel'].includes(action.kind)) return action as EditResult;
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
      return transaction([op]);
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
    const material: Material = { kind: 'media', media: source.media.id };
    if (action.edit === 'append') return transaction([{ edit: 'append', timeline: timeline.id, source: material, ...targets }]);
    const at = marks.in ?? frame;
    if (action.edit === 'replace') material.from = sourceTime(source.frame, source.rate, source.media.facts.duration.scale);
    else if (marks.out !== null && marks.out > at) material.length = marks.out - at;
    const command: Edit = action.edit === 'place_on_top' ? { edit: action.edit, timeline: timeline.id, source: material, at }
      : { edit: action.edit, timeline: timeline.id, source: material, at, ...targets };
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
