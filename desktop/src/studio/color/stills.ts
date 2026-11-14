/**
 * The Gallery of the Color page as the page keeps it (Reference Manual 20, ch. 141–142; the
 * interaction reference, §9.1, §9.2 and §9.4): the albums and their stills as `gallery.list`
 * answers them, listed again whenever the engine says `gallery.changed`; the thumbnails of the
 * stills; what a grab, an Apply Grade and an Append Node Graph send; the settings of the Gallery's
 * menu (Apply Grades Using, Copy Grade: Preserve number of nodes), which are this computer's; and
 * the memories A–H, which the engine does not keep: the page keeps which still each holds, for each
 * project, in the preferences of this computer.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import {
  callEngine, type Clip, type GalleryAlbum, type Group, type KeyframesUsing, type Still, type StillLevel, type Target, type Timeline,
} from '../engine';
import type { GradeLevel } from './clips';

/** Apply Grades Using, in the order of its submenu (p. 3264). No Keyframes is the Gallery's default (§9.4). */
export const KEYFRAMES_USING: readonly KeyframesUsing[] = ['none', 'source_timecode', 'start_frame'];
/** Copy Grade: Preserve number of nodes, 0 to 10 (p. 3285); 0 by default. */
export const PRESERVE_MOST = 10;
/** The memories that keys reach, A to H (Alt+1…8 and Ctrl+1…8, pp. 3271–3272, 3289); the manual's go on to Z. */
export const MEMORY_LETTERS = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H'] as const;
/** The heights of a still's thumbnail that the Still Size slider goes between, in pixels of the page (p. 3263). */
export const STILL_SIZES = { least: 36, most: 160, initial: 63 } as const;
/** The tallest thumbnail that the engine keeps of a still (`gallery.thumbnail`). */
const THUMBNAIL_MOST = 270;

/** What the Gallery's menu sets for every apply and append after it (pp. 3264, 3285). */
export interface ApplySettings { keyframes: KeyframesUsing; preserve: number }
export const DEFAULT_APPLY: ApplySettings = { keyframes: 'none', preserve: 0 };
const APPLY_KEY = 'columbina.studio.gallery.apply';

/** The settings of the Gallery's menu, as this computer keeps them. */
export function readApplySettings(): ApplySettings {
  try {
    const kept = JSON.parse(localStorage.getItem(APPLY_KEY) ?? 'null') as Partial<ApplySettings> | null;
    const keyframes = KEYFRAMES_USING.includes(kept?.keyframes as KeyframesUsing) ? kept!.keyframes! : DEFAULT_APPLY.keyframes;
    const preserve = Number.isInteger(kept?.preserve) && kept!.preserve! >= 0 && kept!.preserve! <= PRESERVE_MOST ? kept!.preserve! : 0;
    return { keyframes, preserve };
  } catch {
    return DEFAULT_APPLY;
  }
}
export function keepApplySettings(settings: ApplySettings): void {
  try { localStorage.setItem(APPLY_KEY, JSON.stringify(settings)); } catch { /* Without storage they hold for this window. */ }
}

/**
 * The memories of a project: the still that each of A–H holds (or null), and the album that the
 * page grabs them into. A memory is a still that keys reach (p. 3271), kept in the project's album
 * of the memories, which the engine lists apart from the albums (`gallery.list`'s `memories`), so
 * that it is not one of the stills of an album; a still dragged onto a memory is copied into it.
 */
export interface Memories { album: string | null; slots: (string | null)[] }
export const NO_MEMORIES: Memories = { album: null, slots: MEMORY_LETTERS.map(() => null) };
const memoriesKey = (project: string) => `columbina.studio.memories:${project}`;

export function readMemories(project: string): Memories {
  try {
    const kept = JSON.parse(localStorage.getItem(memoriesKey(project)) ?? 'null') as Partial<Memories> | null;
    const slots = MEMORY_LETTERS.map((_, at) => {
      const still = Array.isArray(kept?.slots) ? kept!.slots[at] : null;
      return typeof still === 'string' && still ? still : null;
    });
    return { album: typeof kept?.album === 'string' && kept.album ? kept.album : null, slots };
  } catch {
    return NO_MEMORIES;
  }
}
export function keepMemories(project: string, memories: Memories): void {
  try { localStorage.setItem(memoriesKey(project), JSON.stringify(memories)); } catch { /* Without storage they hold for this window. */ }
}

/**
 * The level that a grab takes (p. 3306): the Node Editor's mode for a clip and its group; in the
 * Timeline mode the clip's own grade, as a still holds the timeline's grade besides it whatever the mode.
 */
export const grabLevel = (level: GradeLevel): StillLevel => (level === 'timeline' ? 'clip' : level);

/** What Grab Still sends (Ctrl+Alt+G, p. 3257): into the current album, of the current clip, at the Node Editor's mode. */
export function grabParams(album: string | null, clip: Clip | null, level: GradeLevel): Record<string, unknown> {
  return {
    view: 'timeline',
    ...(album === null ? {} : { album }),
    ...(clip === null ? {} : { clip: clip.id }),
    level: grabLevel(clip !== null && clip.group === null && level !== 'timeline' ? 'clip' : level),
  };
}

/**
 * The grades that an Apply Grade or Append Node Graph goes to (pp. 3287–3290; §9.4): those of the
 * clips chosen in the thumbnail timeline (the current clip where none is), at the Node Editor's
 * mode: the clips' own grades, their groups' Pre-Clip or Post-Clip grades (each group once), or the
 * timeline's grade.
 */
export function applyTargets(level: GradeLevel, timeline: Timeline | null, clips: readonly Clip[], chosen: ReadonlySet<string>, current: Clip | null,
  groups: readonly Group[] = []): Target[] {
  if (timeline === null) return [];
  if (level === 'timeline') return [{ level: 'timeline', timeline: timeline.id }];
  const picked = clips.filter(clip => chosen.has(clip.id));
  const of = picked.length > 0 ? picked : current === null ? [] : [current];
  if (level === 'clip') return of.map(clip => ({ level: 'clip', timeline: timeline.id, clip: clip.id }));
  const known = new Set(groups.map(group => group.id));
  const seen = new Set<string>();
  const targets: Target[] = [];
  for (const clip of of) {
    if (clip.group === null || !known.has(clip.group) || seen.has(clip.group)) continue;
    seen.add(clip.group);
    targets.push({ level, group: clip.group });
  }
  return targets;
}

/**
 * What Apply Grade and Append Node Graph send (`still.apply`): the still, the grades it goes to, and
 * the Gallery's settings: its keyframes, and (of an apply only) the nodes that are preserved.
 */
export function applyParams(still: string, to: readonly Target[], mode: 'apply' | 'append', settings: ApplySettings, revision: number): Record<string, unknown> {
  return {
    still, to: [...to], mode, keyframes: settings.keyframes,
    ...(mode === 'apply' && settings.preserve > 0 ? { preserve_nodes: settings.preserve } : {}),
    revision,
  };
}

/**
 * What Append Node to Selected Clips sends (p. 3297): the current node of the grade shown, after
 * the end of the grades of the clips chosen other than the current one. Its keys go as Apply Grades
 * Using says; with No Keyframes each keyed setting is what it is at the playhead `at` (p. 3393).
 */
export function appendNodeParams(from: Target, node: string, to: readonly Target[], settings: ApplySettings, revision: number, at: number): Record<string, unknown> {
  return { from, node, to: [...to], keyframes: settings.keyframes, ...(settings.keyframes === 'none' ? { at } : {}), revision };
}

/** The clips that Append Node to Selected Clips goes to: those chosen, but the current one (p. 3297, "selected red"). */
export function appendNodeTargets(timeline: Timeline | null, clips: readonly Clip[], chosen: ReadonlySet<string>, current: Clip | null): Target[] {
  if (timeline === null) return [];
  return clips.filter(clip => chosen.has(clip.id) && clip.id !== current?.id).map(clip => ({ level: 'clip', timeline: timeline.id, clip: clip.id }));
}

/** The stills of an album that a search shows: those whose label holds what is searched for, whatever its case (p. 3263). */
export function searched(stills: readonly Still[], search: string): Still[] {
  const asked = search.trim().toLocaleLowerCase();
  return asked === '' ? [...stills] : stills.filter(still => still.label.toLocaleLowerCase().includes(asked));
}

/** The still before or after `current` among those shown (Previous / Next Still, Ctrl+Alt+B / N, p. 3260), stopping at the ends. */
export function stepStill(stills: readonly Still[], current: string | null, direction: -1 | 1): Still | null {
  if (stills.length === 0) return null;
  const at = stills.findIndex(still => still.id === current);
  if (at < 0) return stills[direction > 0 ? 0 : stills.length - 1];
  return stills[Math.max(0, Math.min(stills.length - 1, at + direction))];
}

/** The stills chosen by a click: alone; Ctrl+click added or taken away; Shift+click the range from the one chosen last (p. 3258). */
export function pickStills(shown: readonly Still[], chosen: readonly string[], anchor: string | null, still: string, how: { ctrl: boolean; shift: boolean }): string[] {
  if (how.ctrl) return chosen.includes(still) ? chosen.filter(id => id !== still) : [...chosen, still];
  if (how.shift && anchor !== null) {
    const from = shown.findIndex(each => each.id === anchor);
    const to = shown.findIndex(each => each.id === still);
    if (from >= 0 && to >= 0) return shown.slice(Math.min(from, to), Math.max(from, to) + 1).map(each => each.id);
  }
  return [still];
}

/** The height of thumbnail that is asked for a size of the slider: in steps, so that a drag of it asks for few, and no more than the engine keeps. */
export const thumbnailHeight = (size: number, ratio = 1): number => Math.min(THUMBNAIL_MOST, Math.max(18, Math.ceil((size * ratio) / 18) * 18));

export interface GalleryState {
  albums: GalleryAlbum[];
  /** The project's album of the memories, where there is one. */
  memories: GalleryAlbum | null;
  loaded: boolean;
  /** What the engine said where the Gallery could not be listed. */
  error: string | null;
}

/**
 * The Gallery of the engine that the bridge reaches: listed as it is first shown, and again each
 * time the engine says it changed (`gallery.changed`, told to every client) and when another
 * engine is opened. Thumbnails are kept by still and height: an id is never given twice.
 */
export function useGallery(bridge: Pick<EngineBridge, 'call' | 'onEvent'>, session: number) {
  const [state, setState] = useState<GalleryState>({ albums: [], memories: null, loaded: false, error: null });
  const asking = useRef(0);
  const thumbnails = useRef(new Map<string, Promise<string | null>>());
  const list = useCallback(async () => {
    const asked = ++asking.current;
    try {
      const answer = await callEngine(bridge, 'gallery.list');
      const withStills = (album: GalleryAlbum) => ({ ...album, stills: Array.isArray(album.stills) ? album.stills : [] });
      const albums = Array.isArray(answer?.albums) ? answer.albums.map(withStills) : [];
      const memories = answer?.memories && typeof answer.memories === 'object' ? withStills(answer.memories) : null;
      if (asked === asking.current) setState({ albums, memories, loaded: true, error: null });
    } catch (failure) {
      if (asked === asking.current) setState(old => ({ ...old, loaded: true, error: failure instanceof Error ? failure.message : String(failure) }));
    }
  }, [bridge]);
  useEffect(() => {
    thumbnails.current = new Map();
    void list();
    return bridge.onEvent((event) => {
      if (event.kind === 'notification' && event.method === 'gallery.changed') void list();
      else if (event.kind === 'opened') { thumbnails.current = new Map(); void list(); }
    });
  }, [bridge, session, list]);
  const thumbnail = useCallback((still: string, height: number): Promise<string | null> => {
    const key = `${still}:${height}`;
    let asked = thumbnails.current.get(key);
    if (!asked) {
      asked = callEngine(bridge, 'gallery.thumbnail', { still, height })
        .then(answer => (typeof answer?.png === 'string' ? `data:image/png;base64,${answer.png}` : null), () => null);
      thumbnails.current.set(key, asked);
    }
    return asked;
  }, [bridge]);
  return { ...state, refresh: list, thumbnail };
}
