import { memo, useCallback, useEffect, useRef, useState, type MouseEvent, type PointerEvent } from 'react';
import { Film, Link2 } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Media, Rate } from '../engine';
import type { Thumbnails } from '../media/Thumbnails';
import { rateOf, timecode } from '../timecode';
import { badgesOf, sourceFrameAt, type ColorClip } from './clips';
import { numberText } from './graph';
import { useGradedPicture, type GradedPictures } from './gradedThumbnails';

/** What is written under each thumbnail, double-clicked to go to the next (p. 3028): the codec first, then the clip's name. */
export type ThumbnailText = 'codec' | 'name';

/** How far the pointer goes on a thumbnail before a press becomes a drag through the clip, which picks its picture (p. 3026). */
const DRAG_PX = 4;

/** The badge of a shot match at the foot of a thumbnail: the same grid of dots as on its node. */
function MatchedBadge({ title }: { title: string }) {
  return <svg className="color-badge color-badge-matched" width={12} height={12} viewBox="0 0 12 12" role="img" aria-label={title} data-test="badge-matched">
    <title>{title}</title>
    {[2, 6, 10].flatMap(y => [2, 6, 10].map(x => <circle key={`${x}-${y}`} cx={x} cy={y} r={1.3} />))}
  </svg>;
}

/**
 * The picture of a thumbnail, asked for once it is in view: as its clip is graded where the engine
 * draws it (`graded`, p. 3026), else (or where the engine drew none) a frame of its media; a title
 * or a colour has none but that.
 */
function Picture({ entry, media, rate, cache, offset, graded }: {
  entry: ColorClip; media: Media | undefined; rate: Rate; cache: Thumbnails | null; offset: number; graded?: GradedPictures | null;
}) {
  const box = useRef<HTMLSpanElement>(null);
  const [picture, setPicture] = useState<string | null>(null);
  const [seen, setSeen] = useState(false);
  const { clip } = entry;
  const frame = media?.facts.video ? sourceFrameAt(clip, offset, rate, media) : null;
  const mediaId = clip.source.kind === 'media' ? clip.source.media : null;
  const { picture: gradedPicture, failed } = useGradedPicture(graded, clip, offset, seen);
  // The media's own frame where the engine does not draw the clip graded.
  const ungraded = !graded || failed;
  useEffect(() => {
    if (!graded || seen) return undefined;
    const observer = typeof IntersectionObserver === 'function' ? new IntersectionObserver((shown) => {
      if (shown.some(each => each.isIntersecting)) { setSeen(true); observer?.disconnect(); }
    }) : null;
    if (observer && box.current) observer.observe(box.current);
    else setSeen(true);
    return () => observer?.disconnect();
  }, [graded, seen]);
  useEffect(() => {
    if (!ungraded || cache === null || mediaId === null || frame === null) return undefined;
    let ended = false;
    let asked = false;
    const fetch = () => {
      if (asked || ended) return;
      asked = true;
      void cache.at(mediaId, frame).then(image => { if (!ended) setPicture(image); }, () => {});
    };
    const observer = typeof IntersectionObserver === 'function' ? new IntersectionObserver((seen) => {
      if (seen.some(each => each.isIntersecting)) { fetch(); observer?.disconnect(); }
    }) : null;
    if (observer && box.current) observer.observe(box.current);
    else fetch();
    return () => { ended = true; observer?.disconnect(); };
  }, [cache, mediaId, frame, ungraded]);
  if (gradedPicture) {
    return <span ref={box} className="color-thumb-picture" title={clip.source.kind === 'media' ? media?.name : undefined} data-picture="graded">
      <img src={gradedPicture} alt="" draggable={false} /></span>;
  }
  if (clip.source.kind === 'color') return <span ref={box} className="color-thumb-picture" style={{ background: clip.source.color.slice(0, 7) }} />;
  if (clip.source.kind === 'title') return <span ref={box} className="color-thumb-picture" data-kind="title"><span>{clip.source.text}</span></span>;
  return <span ref={box} className="color-thumb-picture" title={picture === null ? undefined : media?.name}>
    {picture ? <img src={picture} alt="" draggable={false} /> : <Film size={20} aria-hidden="true" />}
  </span>;
}

/** One clip of the thumbnail timeline, with what is written above and below it (pp. 2973–2974, 3027–3028). */
const Thumbnail = memo(function Thumbnail({ entry, media, rate, cache, current, selected, referenced = false, text, offset, onPick, onText, onPicture, graded }: {
  entry: ColorClip; media: Media | undefined; rate: Rate; cache: Thumbnails | null;
  /** The pictures of clips as they are graded, where the engine draws them. */
  graded?: GradedPictures | null;
  current: boolean; selected: boolean; text: ThumbnailText; offset: number;
  /** Whether a timeline wipe shows this clip: outlined in blue (p. 3261). */
  referenced?: boolean;
  onPick: (clip: string, event: MouseEvent | PointerEvent) => void;
  onText: () => void;
  /** The frame of the clip, from its first, that its picture is to show. */
  onPicture: (clip: string, offset: number) => void;
}) {
  const labels = useWords().label.studio.colorPage;
  const { clip, number, trackNumber } = entry;
  const badges = badgesOf(clip);
  const mediaRate = rateOf(media);
  const first = sourceFrameAt(clip, 0, rate, media);
  const press = useRef<{ x: number; scrubbing: boolean } | null>(null);
  const [scrubbed, setScrubbed] = useState<number | null>(null);
  const name = clip.source.kind === 'media' ? media?.name ?? '' : clip.source.kind === 'title' ? clip.source.text : labels.solidColor;
  const codec = clip.source.kind === 'media' ? media?.facts.video?.codec ?? '' : clip.source.kind === 'title' ? labels.title : labels.solidColor;
  const offsetAt = (event: PointerEvent) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    const along = bounds.width > 0 ? (event.clientX - bounds.left) / bounds.width : 0;
    return Math.round(Math.max(0, Math.min(1, along)) * Math.max(0, clip.length - 1));
  };
  return <li role="option" className="color-thumb" aria-selected={selected} aria-current={current || undefined} aria-label={`${numberText(number)} ${name}`}
    tabIndex={current ? 0 : -1}
    data-clip={clip.id} data-number={number} data-current={current || undefined} data-selected={selected || undefined} data-enabled={clip.enabled}
    data-reference={referenced || undefined} title={referenced ? `${name}
${labels.compare.reference}` : name}>
    <div className="color-thumb-top">
      <span className="color-thumb-number" data-graded={badges.graded || undefined} title={badges.graded ? labels.graded : undefined} data-test="thumb-number">
        {numberText(number)}</span>
      <span className="color-thumb-timecode">{first === null ? '' : timecode(first, mediaRate)}</span>
      <span className="color-thumb-track">V{trackNumber}</span>
    </div>
    <div className="color-thumb-frame"
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        press.current = { x: event.clientX, scrubbing: false };
        event.currentTarget.setPointerCapture?.(event.pointerId);
      }}
      onPointerMove={(event) => {
        const held = press.current;
        if (!held) return;
        if (!held.scrubbing && Math.abs(event.clientX - held.x) < DRAG_PX) return;
        held.scrubbing = true;
        setScrubbed(offsetAt(event));
      }}
      onPointerUp={(event) => {
        const held = press.current;
        press.current = null;
        event.currentTarget.releasePointerCapture?.(event.pointerId);
        if (held?.scrubbing) { onPicture(clip.id, offsetAt(event)); setScrubbed(null); } else if (held) onPick(clip.id, event);
      }}
      onPointerCancel={() => { press.current = null; setScrubbed(null); }}>
      <Picture entry={entry} media={media} rate={rate} cache={cache} offset={offset} graded={graded} />
      {scrubbed !== null && <span className="color-thumb-scrub" data-test="thumb-scrub">+{scrubbed}</span>}
    </div>
    <div className="color-thumb-foot">
      <span className="color-thumb-text" onDoubleClick={onText} data-test="thumb-text">{text === 'codec' ? codec : name}</span>
      {badges.grouped && <span className="color-badge" role="img" aria-label={labels.grouped} title={labels.grouped} data-test="badge-group"><Link2 size={11} aria-hidden="true" /></span>}
      {badges.matched && <MatchedBadge title={labels.matched} />}
    </div>
  </li>;
});

/**
 * The thumbnail timeline of the Color page (Reference Manual 20, pp. 2973–2974, 3024–3028): one
 * thumbnail for each clip of the video tracks shown, in the order of their numbers, whatever their
 * length. The current clip is outlined in orange. A click chooses a clip and takes the playhead to
 * its first frame; Ctrl+click and Shift+click choose more without moving it. The wheel scrolls it
 * (up is to the right), and so does a drag with the middle button. Dragged across, a thumbnail
 * goes through its clip, and the frame where it is let go becomes its picture (a matter of the
 * view, which no undo takes back). The clip that a timeline wipe shows is outlined in blue (p. 3261).
 */
export function ThumbnailTimeline({ clips, media, rate, cache, current, selected, onPick, pictures, onPicture, hint, reference = null, graded = null }: {
  clips: readonly ColorClip[];
  media: ReadonlyMap<string, Media>;
  rate: Rate;
  cache: Thumbnails | null;
  current: string | null;
  selected: ReadonlySet<string>;
  onPick: (clip: string, how: { ctrl: boolean; shift: boolean }) => void;
  /** The frame, from its first, that each clip's picture shows where it is not the first. */
  pictures: Readonly<Record<string, number>>;
  onPicture: (clip: string, offset: number) => void;
  hint: string;
  /** The clip that a timeline wipe shows now, where one does. */
  reference?: string | null;
  /** The pictures of the clips as they are graded (p. 3026), where the engine draws them. */
  graded?: GradedPictures | null;
}) {
  const labels = useWords().label.studio.colorPage;
  const list = useRef<HTMLOListElement>(null);
  const [text, setText] = useState<ThumbnailText>('codec');
  const panning = useRef<{ x: number; left: number } | null>(null);
  useEffect(() => {
    const element = list.current;
    if (!element) return undefined;
    const wheel = (event: WheelEvent) => {
      const by = event.deltaY !== 0 ? event.deltaY : -event.deltaX;
      if (by === 0) return;
      event.preventDefault();
      // The wheel turned up goes to the right (p. 3024).
      element.scrollLeft -= by;
    };
    element.addEventListener('wheel', wheel, { passive: false });
    return () => element.removeEventListener('wheel', wheel);
  }, []);
  // The current clip is kept in view as the playhead goes from clip to clip.
  useEffect(() => {
    if (current === null) return;
    const shown = [...list.current?.children ?? []].find(each => each instanceof HTMLElement && each.dataset.clip === current);
    shown?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  }, [current]);
  const told = useRef({ onPick, onPicture });
  told.current = { onPick, onPicture };
  const pick = useCallback((clip: string, event: MouseEvent | PointerEvent) => told.current.onPick(clip, { ctrl: event.ctrlKey || event.metaKey, shift: event.shiftKey }), []);
  const picture = useCallback((clip: string, offset: number) => told.current.onPicture(clip, offset), []);
  const cycle = useCallback(() => setText(old => (old === 'codec' ? 'name' : 'codec')), []);
  if (clips.length === 0) return <p className="caption color-empty" data-test="thumbnails-empty">{hint}</p>;
  return <ol ref={list} className="color-thumbs" role="listbox" aria-multiselectable aria-label={labels.thumbnails} data-test="thumbnails"
    onPointerDown={(event) => {
      if (event.button !== 1) return;
      event.preventDefault();
      panning.current = { x: event.clientX, left: event.currentTarget.scrollLeft };
      event.currentTarget.setPointerCapture?.(event.pointerId);
    }}
    onPointerMove={(event) => { if (panning.current) event.currentTarget.scrollLeft = panning.current.left - (event.clientX - panning.current.x); }}
    onPointerUp={() => { panning.current = null; }}
    onPointerCancel={() => { panning.current = null; }}>
    {clips.map(entry => <Thumbnail key={entry.clip.id} entry={entry} rate={rate} cache={cache}
      media={entry.clip.source.kind === 'media' ? media.get(entry.clip.source.media) : undefined}
      current={entry.clip.id === current} selected={selected.has(entry.clip.id)} referenced={entry.clip.id === reference} text={text} offset={pictures[entry.clip.id] ?? 0}
      onPick={pick} onText={cycle} onPicture={picture} graded={graded} />)}
  </ol>;
}
