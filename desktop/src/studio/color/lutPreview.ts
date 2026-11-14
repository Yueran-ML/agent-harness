/**
 * The pictures of the LUT browser (Reference Manual 20, pp. 2970, 3259, 3410–3411):
 *
 * - each LUT's **thumbnail** on the test image, which the engine draws (`luts.preview`), kept for
 *   each engine while the file and the project's interpolation are as they were;
 * - **Live Preview** (the browser's option menu): over a LUT, the viewer shows the current clip
 *   with that LUT on the current node (`viewer.preview`, which changes nothing of the project),
 *   and the LUT's thumbnail shows the current clip with it too;
 * - **Hover Scrub Preview** (its submenu): as the pointer moves across the thumbnail, the
 *   thumbnail and the viewer, or the thumbnail alone, go through the frames of the current clip;
 *   off, both show the frame at the playhead.
 *
 * Each of the engine's calls is sent one at a time, the latest kept while one is answered, so a
 * pointer that moves quickly asks for no more than the engine can draw.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, type LivePreview, type LutFile, type LutPreview, type Target } from '../engine';

/** How Hover Scrub Preview goes through the current clip: the thumbnail and the viewer, the thumbnail alone, or neither. */
export type HoverScrub = 'both' | 'thumbnail' | 'off';
export const HOVER_SCRUBS: readonly HoverScrub[] = ['both', 'thumbnail', 'off'];

/** The options of the LUT browser's option menu: Live Preview and Hover Scrub Preview. */
export interface LutBrowserOptions { live: boolean; scrub: HoverScrub }

/** Live Preview off (the manual does not say; ours), and no scrubbing: the playhead's frame. */
export const DEFAULT_OPTIONS: LutBrowserOptions = { live: false, scrub: 'off' };

const OPTIONS_KEY = 'columbina-studio-lut-browser';

/** The options as this computer keeps them; the defaults where it keeps none, or something else. */
export function readLutOptions(): LutBrowserOptions {
  try {
    const kept: unknown = JSON.parse(localStorage.getItem(OPTIONS_KEY) ?? 'null');
    if (kept === null || typeof kept !== 'object') return DEFAULT_OPTIONS;
    const { live, scrub } = kept as Partial<LutBrowserOptions>;
    return {
      live: typeof live === 'boolean' ? live : DEFAULT_OPTIONS.live,
      scrub: HOVER_SCRUBS.includes(scrub as HoverScrub) ? scrub as HoverScrub : DEFAULT_OPTIONS.scrub,
    };
  } catch {
    return DEFAULT_OPTIONS;
  }
}

export function keepLutOptions(options: LutBrowserOptions): void {
  try {
    localStorage.setItem(OPTIONS_KEY, JSON.stringify(options));
  } catch {
    // Without storage the options hold for this window.
  }
}

/** The pixels high that the engine draws a thumbnail: twice the browser's, for screens of high density. */
export const THUMBNAIL_HEIGHT = 96;

/** The most LUTs asked of the engine in one call. */
const BATCH = 16;

/** Sends one call at a time, and of those asked for meanwhile the latest alone. */
export class Latest<P> {
  private busy = false;
  private waiting: { params: P } | null = null;

  constructor(private readonly send: (params: P) => Promise<void>) {}

  push(params: P): void {
    if (this.busy) { this.waiting = { params }; return; }
    void this.run(params);
  }

  private async run(params: P): Promise<void> {
    this.busy = true;
    try {
      await this.send(params);
    } catch {
      // A preview that cannot be drawn leaves the picture as it was.
    } finally {
      this.busy = false;
      const next = this.waiting;
      this.waiting = null;
      if (next !== null) void this.run(next.params);
    }
  }
}

/** A thumbnail drawn, as the address of its picture, or none where the engine could not draw it. */
type Drawn = string | null;

const drawnOf = new WeakMap<object, Map<string, Drawn>>();

/** What a thumbnail is kept by: the engine, the file, and how the project reads a cube between its points. */
export function thumbnailKey(session: number, interpolation: string, lut: Pick<LutFile, 'name' | 'size' | 'modified_us'>): string {
  return JSON.stringify([session, interpolation, lut.name, lut.size, lut.modified_us]);
}

function pictureOf(preview: LutPreview): Drawn {
  return 'png' in preview ? `data:image/png;base64,${preview.png}` : null;
}

/**
 * The thumbnails of `luts` on the test image, by `thumbnailKey`, as the engine draws them; asked
 * for as they are shown, those of an engine kept while it is.
 */
export function useLutThumbnails({ bridge, session, native, luts, interpolation }: {
  bridge: Pick<EngineBridge, 'call'>;
  session: number;
  native: boolean;
  luts: readonly LutFile[];
  interpolation: string;
}): ReadonlyMap<string, Drawn> {
  let kept = drawnOf.get(bridge);
  if (!kept) { kept = new Map(); drawnOf.set(bridge, kept); }
  const cache = kept;
  const [, drawn] = useState(0);
  const wanted = luts.filter(lut => lut.status !== 'error' && !lut.shadowed);
  const missing = wanted.filter(lut => !cache.has(thumbnailKey(session, interpolation, lut)));
  const asked = missing.map(lut => thumbnailKey(session, interpolation, lut)).join('\n');
  useEffect(() => {
    if (!native || missing.length === 0) return;
    let gone = false;
    const asking = missing.slice();
    void (async () => {
      for (let at = 0; at < asking.length && !gone; at += BATCH) {
        const batch = asking.slice(at, at + BATCH);
        let previews: LutPreview[] = [];
        try {
          previews = (await callEngine(bridge, 'luts.preview', { luts: batch.map(lut => lut.name), height: THUMBNAIL_HEIGHT })).previews ?? [];
        } catch {
          // An engine that draws none: the LUTs show their mark.
        }
        for (const lut of batch) {
          const preview = previews.find(each => each.lut === lut.name);
          cache.set(thumbnailKey(session, interpolation, lut), preview ? pictureOf(preview) : null);
        }
        if (!gone) drawn(count => count + 1);
      }
    })();
    return () => { gone = true; };
  }, [bridge, session, native, interpolation, asked]);
  return cache;
}

/** Where a LUT would be previewed: the current node of the grade shown, the current clip, and the playhead. */
export interface PreviewPlace {
  grade: Target;
  node: string;
  timeline: string;
  /** The clip whose layers alone a thumbnail shows; none for a grade of the timeline, whose thumbnail is the frame. */
  clip: string | null;
  /** The playhead. */
  frame: number;
  /** The frames of the current clip, which Hover Scrub goes through. */
  start: number;
  length: number;
}

/** The frame of the current clip at `across` (0 to 1) of a thumbnail's width. */
export function scrubbedFrame(place: Pick<PreviewPlace, 'start' | 'length'>, across: number): number {
  const length = Math.max(1, place.length);
  return place.start + Math.min(length - 1, Math.max(0, Math.floor(across * length)));
}

/** The pointer over a LUT: which, and where across its thumbnail (0 to 1). */
interface Hover { lut: string; across: number }

/**
 * Live Preview of the LUT browser: `enter`, `move` and `leave` follow the pointer over the LUTs,
 * and `picture` is the thumbnail of the LUT under it, of the current clip. Nothing is shown where
 * Live Preview is off, there is no current corrector, or no engine shows the viewer.
 */
export function useLivePreview({ bridge, session, native, place, options }: {
  bridge: Pick<EngineBridge, 'call'>;
  session: number;
  native: boolean;
  place: PreviewPlace | null;
  options: LutBrowserOptions;
}) {
  const [hover, setHover] = useState<Hover | null>(null);
  const [picture, setPicture] = useState<{ lut: string; url: string } | null>(null);
  const on = native && options.live && place !== null;
  const viewer = useMemo(() => new Latest<Record<string, unknown>>(async (params) => {
    await callEngine(bridge, 'viewer.preview', params);
  }), [bridge, session]);
  const thumbnail = useMemo(() => new Latest<Record<string, unknown>>(async (params) => {
    const lut = (params.luts as string[])[0];
    const answer = await callEngine(bridge, 'luts.preview', params);
    const drawn = answer.previews?.[0];
    const url = drawn ? pictureOf(drawn) : null;
    setPicture(url === null ? null : { lut, url });
  }), [bridge, session]);
  // What the viewer was last told: whether a preview is on.
  const shown = useRef(false);

  const scrub = options.scrub;
  const wanted: LivePreview | null = on && hover !== null && place !== null
    ? {
      grade: place.grade, node: place.node, lut: hover.lut,
      ...(scrub === 'both' ? { frame: scrubbedFrame(place, hover.across) } : {}),
    }
    : null;
  const wantedKey = JSON.stringify(wanted);
  useEffect(() => {
    if (wanted === null) {
      if (shown.current) { shown.current = false; viewer.push({ on: false }); }
      return;
    }
    shown.current = true;
    viewer.push({ ...wanted });
  }, [wantedKey, viewer]);
  // The thumbnail under the pointer: the current clip with the LUT, at the frame scrubbed to or the playhead.
  const thumbFrame = on && hover !== null && place !== null ? (scrub === 'off' ? place.frame : scrubbedFrame(place, hover.across)) : null;
  const thumbKey = JSON.stringify(on && hover !== null && place !== null
    ? [hover.lut, thumbFrame, place.grade, place.node, place.clip, place.timeline] : null);
  useEffect(() => {
    if (!on || hover === null || place === null || thumbFrame === null) { setPicture(null); return; }
    thumbnail.push({
      luts: [hover.lut], height: THUMBNAIL_HEIGHT,
      frame: { timeline: place.timeline, frame: thumbFrame, ...(place.clip === null ? {} : { clip: place.clip }), grade: place.grade, node: place.node },
    });
  }, [thumbKey, thumbnail]);
  // Left: the viewer shows the project again.
  useEffect(() => () => { if (shown.current) { shown.current = false; void callEngine(bridge, 'viewer.preview', { on: false }).catch(() => {}); } }, [bridge, session]);

  const enter = useCallback((lut: string, across: number) => setHover({ lut, across }), []);
  const move = useCallback((lut: string, across: number) => {
    setHover(old => (old !== null && old.lut === lut && (scrub === 'off' || old.across === across) ? old : { lut, across }));
  }, [scrub]);
  const leave = useCallback(() => setHover(null), []);
  return { on, enter, move, leave, picture: on && hover !== null && picture?.lut === hover.lut ? picture : null };
}
