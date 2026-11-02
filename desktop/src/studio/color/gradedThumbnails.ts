import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Clip, type ColorSettings, type Group, type Media, type Timeline } from '../engine';

/**
 * The thumbnails of the Color page as their clips are graded (Reference Manual 20, p. 3026: they
 * "always show the media as it's currently graded"): each a frame of its clip drawn off screen by
 * the engine (`timeline.thumbnail` with the clip, its layer alone by its own grades), for clips of
 * media, titles and colours alike.
 *
 * A picture is kept by what it depends on (the clip and its grades, its group's, the media and
 * colour management of the project, and the frame); a change of any of those asks for it again
 * once it has held still for a moment (`SETTLE_MS`), so that a drag of a palette does not draw a
 * thumbnail at every step. Update All Thumbnails (p. 3026) forgets them all.
 */
export const SETTLE_MS = 300;
/** The most pictures kept. */
const KEPT = 256;
/**
 * The most pictures asked of the engine at once: it draws them one at a time, with the samples of
 * the pickers (`view.sample` at a node), which so wait behind one or two thumbnails at most.
 */
export const AT_ONCE = 2;

export class GradedThumbnails {
  private readonly images = new Map<string, Promise<string>>();
  private readonly waiting: (() => void)[] = [];
  private asking = 0;
  /** Bumped by Update All Thumbnails, so that every picture is asked again. */
  generation = 0;
  constructor(private readonly bridge: Pick<EngineBridge, 'call'>, private readonly session?: number) {}

  /** The picture of `frame` of the timeline, of `clip`'s layer, for what it depends on (`stamp`). */
  picture(timeline: string, clip: string, frame: number, stamp: string): Promise<string> {
    const key = JSON.stringify([this.generation, timeline, clip, frame, stamp]);
    const existing = this.images.get(key);
    if (existing) return existing;
    const image = this.turn().then(() => callEngine(this.bridge, 'timeline.thumbnail', { timeline, clip, frame, height: 90 },
      this.session === undefined ? undefined : { expectedSession: this.session }))
      .then(result => `data:image/png;base64,${result.png}`)
      .finally(() => this.done());
    this.images.set(key, image);
    void image.catch(() => { if (this.images.get(key) === image) this.images.delete(key); });
    if (this.images.size > KEPT) this.images.delete(this.images.keys().next().value!);
    return image;
  }

  /** Wait for a turn to ask the engine. */
  private turn(): Promise<void> {
    if (this.asking < AT_ONCE) { this.asking += 1; return Promise.resolve(); }
    return new Promise(go => this.waiting.push(() => { this.asking += 1; go(); }));
  }

  private done(): void {
    this.asking -= 1;
    this.waiting.shift()?.();
  }

  /** Forget every picture (Update All Thumbnails). */
  forget(): void {
    this.images.clear();
    this.generation += 1;
  }
}

/** What the graded picture of a clip depends on, besides its frame. */
export function stampOf(clip: Clip, groups: readonly Group[], media: ReadonlyMap<string, Media>, color: ColorSettings | undefined): string {
  const group = clip.group === null ? null : groups.find(each => each.id === clip.group) ?? null;
  const source = clip.source.kind === 'media' ? media.get(clip.source.media) : undefined;
  return JSON.stringify([clip, group, source?.color_override ?? null, source?.identity ?? null, source?.proxy ?? null, color ?? null]);
}

/** What the thumbnail timeline asks of a clip's picture. */
export interface GradedPictures {
  /** The picture of a clip at `offset` frames from its first, as it is graded; null where it cannot be had. */
  picture: (clip: Clip, offset: number) => Promise<string> | null;
  /** What the clip's picture depends on, besides its frame: the picture is asked again as it changes. */
  stamp: (clip: Clip) => string;
}

/** The graded pictures of the clips of `timeline`, and Update All Thumbnails. */
export function useGradedThumbnails({ bridge, session, timeline, groups, media, color, native }: {
  bridge: Pick<EngineBridge, 'call'>; session: number; timeline: Timeline | null; groups: readonly Group[];
  media: ReadonlyMap<string, Media>; color: ColorSettings | undefined;
  /** Whether the engine draws (the desktop's): a page in a browser has no engine to ask. */
  native: boolean;
}) {
  const cache = useMemo(() => new GradedThumbnails(bridge, session), [bridge, session]);
  const [generation, setGeneration] = useState(0);
  const now = useRef({ timeline, groups, media, color });
  now.current = { timeline, groups, media, color };
  const pictures = useMemo<GradedPictures | null>(() => (native ? {
    picture: (clip, offset) => {
      const id = now.current.timeline?.id;
      if (!id) return null;
      return cache.picture(id, clip.id, clip.start + Math.max(0, Math.min(clip.length - 1, offset)),
        stampOf(clip, now.current.groups, now.current.media, now.current.color));
    },
    stamp: clip => `${generation}:${stampOf(clip, now.current.groups, now.current.media, now.current.color)}`,
  } : null), [cache, native, generation, timeline?.id]);
  const updateAll = useCallback(() => { cache.forget(); setGeneration(value => value + 1); }, [cache]);
  return { pictures, updateAll };
}

/**
 * The picture of a thumbnail as it is graded, asked once it is seen and again when what it depends
 * on has settled: the picture before is kept meanwhile.
 */
export function useGradedPicture(pictures: GradedPictures | null | undefined, clip: Clip, offset: number, seen: boolean): string | null {
  const [picture, setPicture] = useState<string | null>(null);
  const stamp = pictures ? pictures.stamp(clip) : '';
  const now = useRef({ pictures, clip, offset });
  now.current = { pictures, clip, offset };
  useEffect(() => {
    if (!pictures || !seen) return undefined;
    let ended = false;
    const timer = setTimeout(() => {
      const asked = now.current.pictures?.picture(now.current.clip, now.current.offset);
      void asked?.then(image => { if (!ended) setPicture(image); }, () => {});
    }, picture === null ? 0 : SETTLE_MS);
    return () => { ended = true; clearTimeout(timer); };
    // The picture before is kept while the next is asked (`picture` is read, not followed).
  }, [pictures, stamp, offset, seen]);
  return pictures ? picture : null;
}
