import { useEffect, useLayoutEffect, useMemo, useRef, useState, type MouseEvent, type PointerEvent } from 'react';
import { useWords } from '../../app/locale';
import type { ClipTrack, Timeline } from '../engine';
import { duration, fitZoom, rulerTicks } from '../timeline/geometry';
import { Scrub } from '../timeline/scrub';
import { timecode } from '../timecode';
import { videoTracks } from './clips';
import { numberText } from './graph';

/** How many video tracks the mini-timeline shows at once; more are scrolled to (p. 2974). */
export const MINI_TRACKS = 6;
/** How wide the column of track numbers is, at the left. */
const NUMBERS_PX = 30;
/** How far one notch of the wheel zooms. */
const WHEEL_ZOOM = 1.25;

/** How a track number stands: white for enabled, grey for disabled, red for enabled but passed over by Up and Down (p. 3025). */
export type TrackState = 'enabled' | 'disabled' | 'skipped';
export const trackState = (track: ClipTrack, skipped: ReadonlySet<string>): TrackState =>
  (!track.enabled ? 'disabled' : skipped.has(track.id) ? 'skipped' : 'enabled');

/**
 * The mini-timeline of the Color page (Reference Manual 20, pp. 2974, 3024–3025): the video tracks
 * of the timeline, each clip as long as it lasts, those on top of each other drawn so, the current
 * clip in orange. A click on a clip chooses it and takes the playhead to its first frame. The ruler
 * shows the timeline's timecode; a click on it takes the playhead there, a drag scrubs. The wheel
 * zooms (up out, down in), about the pointer, never out past the whole timeline, which it shows at
 * first; the bar beneath scrolls it. The numbers of the tracks at the left: a click enables or
 * disables the track (an edit of the project), Alt+click marks it red, to be passed over by Up and
 * Down and hidden from the thumbnail timeline (a state of the view); a pointer on one names the track.
 */
export function MiniTimeline({ timeline, numbers, frame, current, skipped, onClip, onSeek, onEnable, onSkip }: {
  timeline: Timeline;
  /** The number of each clip, as the thumbnails have it. */
  numbers: ReadonlyMap<string, number>;
  frame: number;
  current: string | null;
  skipped: ReadonlySet<string>;
  onClip: (clip: string) => void;
  onSeek: (frame: number, scrubbing: boolean) => void;
  onEnable: (track: ClipTrack) => void;
  onSkip: (track: ClipTrack) => void;
}) {
  const words = useWords().label.studio;
  const labels = words.colorPage;
  const position = words.timeline.position;
  const lanes = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(600);
  const [zoom, setZoom] = useState(1);
  const tracks = useMemo(() => [...videoTracks(timeline)].reverse(), [timeline]);
  const end = useMemo(() => duration(timeline), [timeline]);
  const fitted = fitZoom(end, width);
  const scale = fitted * zoom;
  const now = useRef({ scale, onSeek });
  now.current = { scale, onSeek };
  const scrub = useMemo(() => new Scrub((to, scrubbing) => now.current.onSeek(to, scrubbing)), []);
  const dragging = useRef(false);
  useEffect(() => () => scrub.cancel(), [scrub]);

  useLayoutEffect(() => {
    const element = lanes.current;
    if (!element) return undefined;
    const measure = () => { const seen = element.clientWidth; if (seen > 0) setWidth(seen); };
    measure();
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null;
    observer?.observe(element);
    return () => observer?.disconnect();
  }, []);
  useEffect(() => { setZoom(1); }, [timeline.id]);
  useEffect(() => {
    const element = lanes.current;
    if (!element) return undefined;
    const wheel = (event: WheelEvent) => {
      if (event.deltaY === 0) return;
      event.preventDefault();
      const box = element.getBoundingClientRect();
      const pointer = event.clientX - box.left;
      const { scale: was } = now.current;
      const at = (element.scrollLeft + pointer) / was;
      // Up zooms out, down in (p. 2974), about the frame under the pointer; never out past the whole timeline.
      setZoom((old) => {
        const next = Math.max(1, Math.min(64 / Math.max(fitted, 1e-6), old * (event.deltaY < 0 ? 1 / WHEEL_ZOOM : WHEEL_ZOOM)));
        requestAnimationFrame(() => { element.scrollLeft = Math.max(0, at * fitted * next - pointer); });
        return next;
      });
    };
    element.addEventListener('wheel', wheel, { passive: false });
    return () => element.removeEventListener('wheel', wheel);
  }, [fitted]);

  const frameAt = (event: { clientX: number }) => {
    const element = lanes.current!;
    const box = element.getBoundingClientRect();
    return Math.max(0, Math.min(Math.max(0, end - 1), Math.round((event.clientX - box.left + element.scrollLeft) / scale)));
  };
  const ticks = useMemo(() => rulerTicks(0, end + 1, scale, timeline.rate).filter(tick => tick.major), [end, scale, timeline.rate]);
  const content = Math.max(width, end * scale + 24);
  const pressed = (event: MouseEvent, track: ClipTrack) => (event.altKey ? onSkip(track) : onEnable(track));

  return <div className="color-mini" role="region" aria-label={labels.miniTimeline} data-test="mini-timeline" data-zoom={zoom}>
    <div className="color-mini-numbers" role="group" aria-label={labels.videoTracks} style={{ width: NUMBERS_PX }}>
      <span className="color-mini-corner" />
      {tracks.map((track) => {
        const state = trackState(track, skipped);
        const number = videoTracks(timeline).indexOf(track) + 1;
        return <button key={track.id} type="button" className="color-mini-number" data-track={track.id} data-state={state} title={track.name}
          aria-label={track.name} aria-pressed={state !== 'disabled'} onClick={event => pressed(event, track)}>{number}</button>;
      })}
    </div>
    <div ref={lanes} className="color-mini-lanes">
      <div className="color-mini-content" style={{ width: content }}>
        <div className="color-mini-ruler" role="slider" tabIndex={0} aria-label={position} aria-valuemin={0} aria-valuemax={Math.max(0, end - 1)}
          aria-valuenow={frame} aria-valuetext={timecode(frame, timeline.rate)} data-test="mini-ruler"
          onPointerDown={(event: PointerEvent) => {
            if (event.button !== 0) return;
            event.preventDefault();
            dragging.current = true;
            event.currentTarget.setPointerCapture?.(event.pointerId);
            scrub.drag(frameAt(event));
          }}
          onPointerMove={(event) => { if (dragging.current) scrub.drag(frameAt(event)); }}
          onPointerUp={(event) => {
            if (!dragging.current) return;
            dragging.current = false;
            event.currentTarget.releasePointerCapture?.(event.pointerId);
            scrub.finish(frameAt(event));
          }}
          onPointerCancel={() => { dragging.current = false; scrub.cancel(); }}>
          {ticks.map(tick => <span key={tick.frame} className="color-mini-tick" style={{ left: tick.frame * scale }} data-label={timecode(tick.frame, timeline.rate)} />)}
        </div>
        {tracks.map(track => <div key={track.id} className="color-mini-track" data-track={track.id} data-state={trackState(track, skipped)}>
          {track.clips.map(clip => <button key={clip.id} type="button" className="color-mini-clip" data-clip={clip.id} data-current={clip.id === current || undefined}
            data-enabled={clip.enabled} tabIndex={-1} aria-label={numberText(numbers.get(clip.id) ?? 0)} style={{ left: clip.start * scale, width: Math.max(2, clip.length * scale) }}
            onClick={() => onClip(clip.id)} />)}
        </div>)}
        <span className="color-mini-playhead" style={{ left: frame * scale }} aria-hidden="true" />
      </div>
    </div>
  </div>;
}
