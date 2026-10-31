/**
 * What the viewer of the Color page compares its frame with, worked out (Reference Manual 20,
 * pp. 2999–3006, 3260–3261; the interaction reference, §9.3): the whole wipe that each change
 * sends again, where a drag on the picture puts its border, the reference of a timeline wipe with
 * Gang on and off, the items of each split screen mode, and which tile of the grid a point is in.
 */
import type { Clip, ViewerReference, WipeSettings, WipeStyle } from '../engine';

/** The wipe styles of the Wipe Style submenu, in its order (p. 3000); Alpha needs the alpha output of the node graph and is not done. */
export const WIPE_STYLES: readonly WipeStyle[] = ['horizontal', 'vertical', 'diagonal', 'mix', 'difference', 'box', 'venetian_blind', 'checkerboard'];
/** A wipe as it begins: Horizontal, its border halfway (the interaction reference, §9.3). */
export const DEFAULT_WIPE: WipeSettings = { style: 'horizontal', position: 0.5, angle: 45, invert: false };

/** Reference Wipe Mode (pp. 3001–3002): a still of the Gallery (the default) or a clip of the timeline; Offline is not done. */
export type ReferenceMode = 'gallery' | 'timeline';

/**
 * The split screen modes that the engine can show (p. 3003–3006): those of pictures that are there
 * already. Playheads needs the playheads A–D of the mini-timeline, which are not done (§1.4); those
 * of grades, LUTs and versions applied to the current clip are not done by the engine.
 */
export type SplitMode = 'selected_clips' | 'selected_stills' | 'neighbor_clips' | 'current_group';
export const SPLIT_MODES: readonly SplitMode[] = ['current_group', 'neighbor_clips', 'selected_clips', 'selected_stills'];
/** The mode that Ctrl+Alt+W turns on the first time (§9.3): Selected Clips. */
export const DEFAULT_SPLIT: SplitMode = 'selected_clips';
/** The most pictures that a split screen shows (p. 3005). */
export const SPLIT_MOST = 16;

const clamp = (value: number, least: number, most: number) => Math.min(most, Math.max(least, value));

/** The cursor of the picture while a wipe of a style can be dragged on it. */
export function wipeCursor(style: WipeStyle): 'resize_ew' | 'resize_ns' | 'move' | null {
  if (style === 'difference') return null;
  if (style === 'vertical' || style === 'venetian_blind') return 'resize_ns';
  if (style === 'diagonal' || style === 'box') return 'move';
  return 'resize_ew';
}

/**
 * The `position` of a wipe that puts its border at the point (x, y) of a frame `width` × `height`
 * (p. 3000): a straight border goes through the point, across its normal, as the engine draws it
 * (1 is all A, 0 all B); a box reaches out to the point; a mix and the checks go left to right, the
 * strips of a Venetian Blind up and down. A difference has nothing to drag: null.
 */
export function wipePositionAt(wipe: WipeSettings, x: number, y: number, width: number, height: number): number | null {
  if (width <= 0 || height <= 0) return null;
  const from = [x - width / 2, y - height / 2];
  switch (wipe.style) {
    case 'difference': return null;
    case 'horizontal': return clamp(x / width, 0, 1);
    case 'vertical':
    case 'venetian_blind': return clamp(y / height, 0, 1);
    case 'diagonal': {
      const radians = (wipe.angle * Math.PI) / 180;
      const normal = [Math.cos(radians), Math.sin(radians)];
      const extent = Math.abs(normal[0]) * width + Math.abs(normal[1]) * height;
      return extent > 0 ? clamp((from[0] * normal[0] + from[1] * normal[1]) / extent + 0.5, 0, 1) : null;
    }
    case 'box': return clamp(Math.max(Math.abs(from[0]) / (width / 2), Math.abs(from[1]) / (height / 2)), 0, 1);
    default: return clamp(x / width, 0, 1);
  }
}

/**
 * The angle of a diagonal border that points from the centre of the frame to (x, y), in degrees
 * clockwise from upright (Alt+drag "in a circle", p. 3000); null at the centre itself.
 */
export function wipeAngleAt(x: number, y: number, width: number, height: number): number | null {
  const dx = x - width / 2;
  const dy = y - height / 2;
  if (dx === 0 && dy === 0) return null;
  const degrees = (Math.atan2(dx, -dy) * 180) / Math.PI;
  return Math.round(degrees * 10) / 10;
}

/** The whole of a wipe as `viewer.wipe` takes it, with its reference: every setting each time. */
export function wipeParams(wipe: WipeSettings, reference: ViewerReference): Record<string, unknown> {
  return { on: true, style: wipe.style, position: wipe.position, angle: wipe.angle, invert: wipe.invert, reference };
}

/**
 * A wipe of the timeline (Wipe Timeline Clip, p. 3002): the clip right-clicked, at the frame of it
 * that is as far from its first as the playhead is from the current clip's, moved by the frames
 * that Ctrl+Alt+Left and Right step it (`shift`).
 */
export interface TimelineWipe { clip: string; frame: number }

/**
 * A wipe of the timeline as the engine is asked for it. Gang off: the clip at its frame, which
 * stays where it is whatever clip is current. Gang on (p. 3002): the clip as far from the current
 * one as it was when the wipe began, at the playhead's offset into the current clip moved as far as
 * it was then; the clip just before or after, unmoved, as `neighbour`, which follows the playhead
 * as it plays. Null where the ganged clip is beyond the first or the last.
 */
export function timelineReference(wipe: TimelineWipe, gang: { distance: number; shift: number } | null, clips: readonly Clip[],
  current: Clip | null, frame: number): ViewerReference | null {
  if (gang === null || current === null) {
    const clip = clips.find(each => each.id === wipe.clip);
    return clip ? { kind: 'clip', clip: clip.id, frame: clamp(wipe.frame, 0, Math.max(0, clip.length - 1)) } : null;
  }
  const at = clips.findIndex(each => each.id === current.id);
  const clip = at < 0 ? undefined : clips[at + gang.distance];
  if (!clip) return null;
  if (gang.shift === 0 && Math.abs(gang.distance) === 1) return { kind: 'neighbour', side: gang.distance < 0 ? 'previous' : 'next', current: current.id };
  return { kind: 'clip', clip: clip.id, frame: clamp(frame - current.start + gang.shift, 0, Math.max(0, clip.length - 1)) };
}

/** The clip of the timeline that a reference shows now, where the engine answered its frame (`at`): to be outlined in blue (p. 3261). */
export function referenceClip(clips: readonly Clip[], at: number | null | undefined, named?: string): string | null {
  if (at === null || at === undefined) return null;
  const covering = clips.filter(clip => clip.start <= at && at < clip.start + clip.length);
  return (covering.find(clip => clip.id === named) ?? covering.at(-1))?.id ?? null;
}

/** What a split screen of a mode shows, and which of its tiles is the current one. */
export interface SplitItems { items: ViewerReference[]; selected: number; clips: (string | null)[] }

/**
 * The pictures of a split screen mode (pp. 3004–3006), in reading order, at most 16:
 * - Selected Clips: the clips chosen in the thumbnail timeline (the current one where none is), the
 *   current one at the playhead and each other at its first frame;
 * - Selected Still Images: the current clip, then the stills chosen in the Gallery;
 * - Neighbor Clips: the two clips before the current one above, the current one and the next below;
 *   those that are not there (before the first, after the last) are left out;
 * - Current Group: the clips of the current clip's group, or the current clip where it has none.
 */
export function splitItems(mode: SplitMode, { clips, chosen, current, stills, frame }: {
  clips: readonly Clip[]; chosen: ReadonlySet<string>; current: Clip | null; stills: readonly string[]; frame: number;
}): SplitItems {
  const ofClips = (shown: readonly Clip[]): SplitItems => {
    const kept = shown.slice(0, SPLIT_MOST);
    const items = kept.map((clip): ViewerReference => (clip.id === current?.id ? { kind: 'current' } : { kind: 'clip', clip: clip.id, frame: 0 }));
    return { items, selected: Math.max(0, kept.findIndex(clip => clip.id === current?.id)), clips: kept.map(clip => clip.id) };
  };
  const alone = (): SplitItems => ({ items: [{ kind: 'current' }], selected: 0, clips: [current?.id ?? null] });
  switch (mode) {
    case 'selected_clips': {
      const picked = clips.filter(clip => chosen.has(clip.id));
      return picked.length > 0 ? ofClips(picked) : alone();
    }
    case 'selected_stills': {
      const kept = stills.slice(0, SPLIT_MOST - 1);
      return { items: [{ kind: 'current' }, ...kept.map((still): ViewerReference => ({ kind: 'still', still }))], selected: 0, clips: [current?.id ?? null, ...kept.map(() => null)] };
    }
    case 'neighbor_clips': {
      const at = current === null ? -1 : clips.findIndex(clip => clip.id === current.id);
      if (at < 0) return alone();
      const offset = current === null ? 0 : frame - current.start;
      const items: ViewerReference[] = [];
      const shown: (string | null)[] = [];
      const before = clips[at - 2];
      if (before) { items.push({ kind: 'clip', clip: before.id, frame: clamp(offset, 0, Math.max(0, before.length - 1)) }); shown.push(before.id); }
      if (clips[at - 1]) { items.push({ kind: 'neighbour', side: 'previous', current: current!.id }); shown.push(clips[at - 1].id); }
      items.push({ kind: 'current' }); shown.push(current!.id);
      const selected = items.length - 1;
      if (clips[at + 1]) { items.push({ kind: 'neighbour', side: 'next', current: current!.id }); shown.push(clips[at + 1].id); }
      return { items, selected, clips: shown };
    }
    case 'current_group': {
      if (current === null || current.group === null) return alone();
      return ofClips(clips.filter(clip => clip.group === current.group));
    }
    default: return alone();
  }
}

/** The grid of a split screen as the engine lays it (PROTOCOL.md): as many columns as √n rounded up, as many rows as needed. */
export function splitGrid(count: number): { columns: number; rows: number } {
  if (count <= 0) return { columns: 0, rows: 0 };
  const columns = Math.ceil(Math.sqrt(count));
  return { columns, rows: Math.ceil(count / columns) };
}

/** The tile of a split screen of `count` pictures that the point (x, y) of a frame `width` × `height` is in, in reading order; null outside them. */
export function tileAt(count: number, x: number, y: number, width: number, height: number): number | null {
  const { columns, rows } = splitGrid(count);
  if (columns === 0 || x < 0 || y < 0 || x >= width || y >= height) return null;
  // Each cell is cut at whole pixels, as the engine cuts them: the edge of the i-th of n is i · length / n, rounded down.
  const cellOf = (at: number, of: number, length: number) => {
    const pixel = Math.floor(at);
    let cell = 0;
    while (cell < of - 1 && pixel >= Math.floor(((cell + 1) * Math.floor(length)) / of)) cell += 1;
    return cell;
  };
  const tile = cellOf(y, rows, height) * columns + cellOf(x, columns, width);
  return tile < count ? tile : null;
}
