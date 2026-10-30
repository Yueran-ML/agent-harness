import type { Key } from '../../app/keys';
import type { Media, Monitor, Op, Rate, Time, Timeline, Transition } from '../engine';
import { duration, snapPoints } from './geometry';
import type { Change, Edit, Material, Targets, Transaction } from './commands';
import { clearing, type Tool } from './interact';
import { clipLocations, linkedClips, primaryTrack } from './selection';
import type { Marks } from './snap';
import { materialLength } from './drop';
import { effectTransaction } from '../effects/drop';

/**
 * The edits that put the source on the timeline. Those of the Cut page: `smart_insert` (at the
 * edit point nearest the playhead), `ripple_overwrite` (in place of the clip at the playhead, the
 * rest rippled by the difference) and `source_overwrite` (on top, in sync by timecode: as the
 * engine reads no timecode of media, it does nothing, as DaVinci Resolve does without overlapping
 * timecode, Reference Manual 20, p. 563).
 */
export type PlaceEdit = 'insert' | 'overwrite' | 'replace' | 'place_on_top' | 'append' | 'smart_insert' | 'ripple_overwrite' | 'source_overwrite';
export type QuickTransition = 'dissolve' | 'dip_black';
export type EditAction =
  | { kind: 'tool'; tool: Tool }
  | { kind: 'snap' | 'linked' | 'fit' | 'cancel' | 'speed' | 'enabled' | 'link' | 'transition' | 'blade' }
  /** Lock or unlock a track by its number among those of its kind (V1 is 1), or all tracks of that kind (`number` null). */
  | { kind: 'lock'; tracks: 'video' | 'audio'; number: number | null }
  | { kind: 'history'; method: 'history.undo' | 'history.redo' }
  | { kind: 'mark'; edge: 'in' | 'out' | 'clear' }
  | { kind: 'place'; edit: PlaceEdit }
  /** A transition of picture at the edit point nearest the playhead, as the Cut page's buttons add one. */
  | { kind: 'quick'; transition: QuickTransition }
  | { kind: 'delete'; ripple: boolean }
  | { kind: 'cut'; direction: -1 | 1 }
  | { kind: 'zoom'; direction: -1 | 1 };

/** Classify first so modified shortcuts cannot fall through to their unmodified tool or fit key. */
export function editKey(key: Key): EditAction | null {
  if (key.composing || key.repeat && !['ArrowUp', 'ArrowDown'].includes(key.key)) return null;
  const value = key.key.length === 1 ? key.key.toLowerCase() : key.key;
  const control = key.ctrl || key.meta;
  if (control) {
    // Ctrl+Alt+L links the clips chosen, or unlinks them (Linked Clips, Reference Manual 20, pp. 771, 1097).
    if (value === 'l' && key.alt && !key.shift) return { kind: 'link' };
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
  if (key.alt && key.shift) {
    // Alt+Shift+1 to 8 lock or unlock V1 to V8, Alt+Shift+9 all video tracks, Alt+Shift+F1 to F8
    // A1 to A8, Alt+Shift+F9 all audio tracks (Reference Manual 20, p. 762). A digit is known by
    // where it lies: with Shift it types another sign.
    const digit = /^Digit([1-9])$/.exec(key.code ?? '')?.[1] ?? (/^[1-9]$/.test(value) ? value : null);
    if (digit !== null) return { kind: 'lock', tracks: 'video', number: digit === '9' ? null : Number(digit) };
    const f = /^F([1-9])$/.exec(value)?.[1];
    return f ? { kind: 'lock', tracks: 'audio', number: f === '9' ? null : Number(f) } : null;
  }
  if (key.alt) return value === 'x' ? { kind: 'mark', edge: 'clear' } : null;
  if (key.shift) {
    if (value === 'z') return { kind: 'fit' };
    if (value === 'F12') return { kind: 'place', edit: 'append' };
    if (value === 'F10') return { kind: 'place', edit: 'ripple_overwrite' };
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
  /**
   * Whether the keys are those of the Cut page: there F9 is Smart Insert (F10 stays Overwrite,
   * Reference Manual 20, p. 565), and Ctrl+T puts a dissolve at the edit point nearest the playhead.
   */
  cut?: boolean;
}
export type EditResult =
  | { kind: 'transaction'; transaction: Transaction }
  | { kind: 'history'; method: 'history.undo' | 'history.redo' }
  | { kind: 'seek'; monitor: Monitor; frame: number }
  | { kind: 'marks'; marks: Marks }
  | { kind: 'speed'; clips: string[] }
  | { kind: 'tool'; tool: Tool }
  | { kind: 'snap' | 'linked' | 'fit' | 'cancel' }
  | { kind: 'zoom'; direction: -1 | 1 }
  /** The key was for this page, and there is nothing to do: a transition of that kind and length is there already. */
  | { kind: 'none' };

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

/** How near the playhead is to an edit point, in frames, for Place On Top on the Cut page to go there (Reference Manual 20, p. 563). */
export const PLACE_ON_TOP_FRAMES = 5;

const compare = (a: Time, b: Time) => BigInt(a.ticks) * BigInt(b.scale) - BigInt(b.ticks) * BigInt(a.scale);

/**
 * The edit point nearest `frame` that Smart Insert goes in at, as the engine finds it: the starts
 * and ends of the clips on the tracks of picture and sound that are not locked, and the start of
 * the timeline; of two as near, the earlier.
 */
export function nearestCut(timeline: Timeline, frame: number): number {
  let best = 0;
  for (const track of timeline.tracks) {
    if (track.kind === 'subtitle' || track.locked) continue;
    for (const clip of track.clips) for (const cut of [clip.start, clip.start + clip.length]) {
      const distance = Math.abs(cut - frame);
      const old = Math.abs(best - frame);
      if (distance < old || (distance === old && cut < best)) best = cut;
    }
  }
  return best;
}

/** Two clips that meet at an edit point on a track of picture, and the transition already there. */
export interface Meeting { track: string; left: string; right: string; existing: Transition | null }

/**
 * The edit point nearest `frame` where two clips meet on a track of picture that is not locked (of
 * two as near, the earlier), and the clips that meet there on each such track; null where none do.
 */
export function nearestEditPoint(timeline: Timeline, frame: number): { at: number; meetings: Meeting[] } | null {
  let at: number | null = null;
  const meetings: (Meeting & { at: number })[] = [];
  for (const track of timeline.tracks) {
    if (track.kind !== 'video' || track.locked) continue;
    for (let i = 1; i < track.clips.length; i++) {
      const left = track.clips[i - 1];
      const right = track.clips[i];
      if (left.start + left.length !== right.start) continue;
      const existing = track.transitions.find(each => each.left === left.id && each.right === right.id) ?? null;
      meetings.push({ track: track.id, left: left.id, right: right.id, existing, at: right.start });
      const distance = Math.abs(right.start - frame);
      if (at === null || distance < Math.abs(at - frame) || (distance === Math.abs(at - frame) && right.start < at)) at = right.start;
    }
  }
  return at === null ? null : { at, meetings: meetings.filter(each => each.at === at).map(({ at: _, ...meeting }) => meeting) };
}

/**
 * The edits that empty the frames `from`–`to` of the video and audio tracks that are not locked,
 * leaving the rest where it is (a lift of a range): clips inside are lifted, those across an end
 * are trimmed back to it, and one across both is split at the end and its first part trimmed to
 * the start. Each edit names clips as they are before it: a split keeps its id on the first part.
 */
export function liftRange(timeline: Timeline, from: number, to: number): Edit[] {
  return timeline.tracks.flatMap(track => track.kind === 'subtitle' || track.locked ? [] : clearing(timeline.id, track.clips, [[from, to]], track.id));
}

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
  // The clips chosen whose settings may be changed: not those on locked tracks, linked or not
  // (Reference Manual 20, p. 762); basic operations would change them all the same.
  const open = selected.filter(id => !locations.get(id)!.track.locked);
  const activeTracks = Object.values(targets).filter((id): id is string => typeof id === 'string');
  if (action.kind === 'quick') {
    // One transition of the length and alignment that DaVinci Resolve gives a standard one, on each
    // track of picture where clips meet at that edit point; one there already becomes of this kind.
    const point = nearestEditPoint(timeline, frame);
    if (!point) return null;
    return transaction(point.meetings.flatMap(meeting => effectTransaction(timeline, revision, action.transition,
      { kind: 'cut', track: meeting.track, left: meeting.left, right: meeting.right, cut: point.at, align: 'center', existing: meeting.existing }, '').ops)) ?? { kind: 'none' };
  }
  if (action.kind === 'place') {
    if (!source) return null;
    const edit = context.cut && action.edit === 'insert' ? 'smart_insert' : action.edit;
    // Source Overwrite lines the source up with the timeline by its timecode. The engine reads no
    // timecode of media, so there is never any to overlap, and, as DaVinci Resolve says of that
    // case, "this edit does nothing" (Reference Manual 20, p. 563): its button is disabled.
    if (edit === 'source_overwrite') return null;
    const whole: Material = { kind: 'media', media: source.media.id };
    if (edit === 'append') return transaction([{ edit: 'append', timeline: timeline.id, source: whole, ...targets }]);
    // Smart Insert goes in at the edit point nearest the playhead, which the page shows; it is sent
    // as that point, and the engine, which finds the nearest one again, keeps it.
    if (edit === 'smart_insert') return transaction([{ edit, timeline: timeline.id, source: whole, at: nearestCut(timeline, frame), ...targets }]);
    // With timeline In and Out points, Ripple Overwrite puts the source in place of what lies between
    // them, the rest moving by the difference (Reference Manual 20, pp. 561, 842): the range is taken
    // out and the source inserted at the In, in one transaction. Without, the clip at the playhead.
    // Clips chosen do not change this, as they do for the keys of deleting (p. 760): the manual
    // describes this edit by the range alone.
    if (edit === 'ripple_overwrite' && marks.in !== null && marks.out !== null && marks.out > marks.in) {
      return transaction([
        { edit: 'ripple_delete_range', timeline: timeline.id, from: marks.in, to: marks.out },
        { edit: 'insert', timeline: timeline.id, source: whole, at: marks.in, ...targets },
      ]);
    }
    if (edit === 'ripple_overwrite') return transaction([{ edit, timeline: timeline.id, source: whole, at: frame, ...targets }]);
    // Place On Top on the Cut page, without timeline In or Out points: a playhead within five frames
    // of an edit point puts the source at that edit point, the one Smart Insert marks (Reference
    // Manual 20, p. 563); else at the playhead.
    const near = context.cut && edit === 'place_on_top' && marks.in === null && marks.out === null ? nearestCut(timeline, frame) : frame;
    const placed = threePoint(timeline, source, marks, Math.abs(near - frame) <= PLACE_ON_TOP_FRAMES ? near : frame, edit === 'replace');
    const command: Edit = edit === 'place_on_top' ? { edit, timeline: timeline.id, source: placed.material, at: placed.at }
      : { edit, timeline: timeline.id, source: placed.material, at: placed.at, ...targets };
    return transaction([command]);
  }
  if (action.kind === 'delete') {
    if (!selected.length && marks.in !== null && marks.out !== null && marks.out > marks.in) {
      // With timeline In and Out points and no clip chosen, the keys of deleting take the range on
      // the tracks that are not locked: Delete closes it, Backspace leaves it empty (Reference
      // Manual 20, p. 765). Clips chosen by hand come first: "manual selections almost always take
      // precedence over automatic selections you define using the In and Out points" (p. 760).
      return action.ripple ? transaction([{ edit: 'ripple_delete_range', timeline: timeline.id, from: marks.in, to: marks.out }])
        : transaction(liftRange(timeline, marks.in, marks.out));
    }
    if (!selected.length) return null;
    // On the Cut page, a clip of Track 1 deleted leaves no gap: "the Timeline will ripple to close
    // the gap automatically" (Reference Manual 20, p. 588). The others go as on the Edit page.
    const primary = context.cut && !action.ripple ? primaryTrack(timeline) : null;
    const first = primary ? selected.filter(id => primary.clips.has(id)) : [];
    if (first.length > 0) {
      const rest = selected.filter(id => !first.includes(id));
      return transaction([
        ...(rest.length > 0 ? [{ edit: 'lift' as const, timeline: timeline.id, clips: rest, linked: false }] : []),
        { edit: 'ripple_delete', timeline: timeline.id, clips: first, linked },
      ]);
    }
    return transaction([{ edit: action.ripple ? 'ripple_delete' : 'lift', timeline: timeline.id, clips: selected, linked }]);
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
  if (action.kind === 'speed') return open.length ? { kind: 'speed', clips: open } : null;
  if (action.kind === 'enabled') {
    const enabled = open.every(id => !locations.get(id)!.clip.enabled);
    return transaction(open.map(id => ({ op: 'set_clip', timeline: timeline.id, id, enabled })));
  }
  if (action.kind === 'lock') {
    const ofKind = timeline.tracks.filter(track => track.kind === action.tracks);
    const tracks = action.number === null ? ofKind : ofKind.slice(action.number - 1, action.number);
    const locked = !tracks.every(track => track.locked);
    return transaction(tracks.map(track => ({ op: 'set_track', timeline: timeline.id, id: track.id, locked })));
  }
  if (action.kind === 'link') {
    // Clips chosen that are one group already are unlinked; else they are linked into a new one.
    const chosen = open.map(id => locations.get(id)!.clip);
    const groups = new Set(chosen.map(clip => clip.link));
    if (groups.size === 1 && !groups.has(null)) return transaction(chosen.map(clip => ({ op: 'set_clip', timeline: timeline.id, id: clip.id, link: null })));
    // The first names the new group, and the others join it (an empty link would make a group for each).
    return chosen.length >= 2 ? transaction(chosen.map(clip => ({ op: 'set_clip', timeline: timeline.id, id: clip.id, link: '$link' }))) : null;
  }
  if (action.kind === 'transition') {
    if (context.cut) return editCommand({ kind: 'quick', transition: 'dissolve' }, context);
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
