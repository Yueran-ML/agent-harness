import { useCallback, useEffect, useMemo, useRef, type PointerEvent } from 'react';
import { useWords } from '../../app/locale';
import type { EngineBridge } from '../../bridge';
import { callEngine, type Media, type Rate } from '../engine';
import { Scrub } from '../timeline/scrub';
import { framesOf, rateOf } from '../timecode';

/** How long a still lasts on the tape, as it lasts on a timeline: DaVinci Resolve's standard 5 seconds. */
const STILL_SECONDS = 5;

/** One medium on the tape: its frames at its own rate, and where it lies, in frames and in seconds. */
export interface TapeSegment {
  media: Media;
  rate: Rate;
  frames: number;
  /** The frames of the media before it on the tape, each counted at its own rate. */
  offset: number;
  /** Where it starts on the tape, and how long it lasts there, in seconds. */
  start: number;
  seconds: number;
}
export interface Tape { segments: TapeSegment[]; frames: number; seconds: number }

/**
 * The source tape: the media of a bin, one after another, as one long tape. Each keeps its own
 * frames, at its own rate, and its own marks; the tape only says where each lies. It is laid out
 * by time, so that a second of one medium is as long as a second of another.
 */
export function tapeOf(media: readonly Media[], timelines: readonly { rate: Rate }[] = []): Tape {
  const segments: TapeSegment[] = [];
  let offset = 0;
  let start = 0;
  for (const item of media) {
    const rate = rateOf(item, timelines);
    const seconds = item.facts.video?.still ? STILL_SECONDS : Math.max(0, item.facts.duration.ticks / item.facts.duration.scale);
    const frames = Math.max(1, item.facts.video?.still ? Math.round(STILL_SECONDS * rate.num / rate.den) : framesOf(item.facts.duration, rate));
    segments.push({ media: item, rate, frames, offset, start, seconds: seconds > 0 ? seconds : frames * rate.den / rate.num });
    offset += frames;
    start += segments.at(-1)!.seconds;
  }
  return { segments, frames: offset, seconds: start };
}

/** The medium and its frame at a frame of the tape, which is never between two media: a frame is of one of them. */
export function placeOnTape(tape: Tape, index: number): { media: string; frame: number } | null {
  if (tape.segments.length === 0) return null;
  const at = Math.max(0, Math.min(tape.frames - 1, Math.floor(index)));
  const segment = [...tape.segments].reverse().find(each => each.offset <= at)!;
  return { media: segment.media.id, frame: at - segment.offset };
}

/** The frame of the tape at `seconds` into it: in the medium that lies there, at its own rate. */
export function tapeIndexAt(tape: Tape, seconds: number): number {
  if (tape.segments.length === 0) return 0;
  const at = Math.max(0, Math.min(tape.seconds, seconds));
  const segment = tape.segments.find(each => at < each.start + each.seconds) ?? tape.segments.at(-1)!;
  const frame = Math.min(segment.frames - 1, Math.floor((at - segment.start) / segment.seconds * segment.frames));
  return segment.offset + Math.max(0, frame);
}

/** The medium before or after `media` on the tape; the first where `media` is not on it. */
export function tapeStep(tape: Tape, media: string | null, direction: -1 | 1): string | null {
  const at = tape.segments.findIndex(each => each.media.id === media);
  if (at < 0) return tape.segments[0]?.media.id ?? null;
  return tape.segments[at + direction]?.media.id ?? null;
}

/**
 * Takes the source monitor to a frame of a medium of the tape: the medium is loaded first where the
 * monitor has another, and the frame sought after it, in the order asked.
 */
export function useTapeGo(bridge: Pick<EngineBridge, 'call'>, loaded: string | null, onFailure: (failure: unknown) => void) {
  const current = useRef<{ media: string | null; pending: Promise<unknown> }>({ media: loaded, pending: Promise.resolve() });
  const failed = useRef(onFailure);
  failed.current = onFailure;
  useEffect(() => { current.current.media = loaded; }, [loaded]);
  return useCallback((media: string, frame: number, scrubbing: boolean) => {
    const state = current.current;
    const seek = () => callEngine(bridge, 'transport.seek', { monitor: 'source', frame, scrubbing });
    if (state.media !== media) {
      state.media = media;
      const load = callEngine(bridge, 'transport.load', { monitor: 'source', media });
      state.pending = (frame > 0 || !scrubbing ? load.then(seek) : load).catch(failure => failed.current(failure));
    } else state.pending = state.pending.then(seek).catch(failure => failed.current(failure));
  }, [bridge]);
}

const percent = (value: number, whole: number) => `${whole > 0 ? value / whole * 100 : 0}%`;

/**
 * The strip of the source tape under the viewer: the media of the bin end to end, each named, with
 * its own marks, the boundaries between them, and the playhead in the medium that the viewer has.
 * Dragged, it goes through the media, loading each as it is reached.
 */
export function SourceTape({ tape, loaded, frame, onGo }: {
  tape: Tape;
  /** The medium that the source monitor has, and its frame. */
  loaded: string | null; frame: number | null;
  onGo: (media: string, frame: number, scrubbing: boolean) => void;
}) {
  const words = useWords();
  const strip = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  const now = useRef({ tape, onGo });
  now.current = { tape, onGo };
  const scrub = useMemo(() => new Scrub((index, scrubbing) => {
    const place = placeOnTape(now.current.tape, index);
    if (place) now.current.onGo(place.media, place.frame, scrubbing);
  }), []);
  useEffect(() => () => scrub.cancel(), [scrub]);
  const indexAt = (event: PointerEvent) => {
    const box = strip.current!.getBoundingClientRect();
    return tapeIndexAt(tape, (event.clientX - box.left) / Math.max(1, box.width) * tape.seconds);
  };
  const here = tape.segments.find(each => each.media.id === loaded);
  const head = here && frame !== null ? here.start + Math.min(1, (frame + 0.5) / here.frames) * here.seconds : null;
  if (tape.segments.length === 0) return <p className="caption source-tape-none" data-test="tape-none">{words.hint.studio.cutPage.emptyBin}</p>;
  return <div ref={strip} className="source-tape" role="group" aria-label={words.label.studio.cutPage.tapeStrip} data-test="source-tape"
    onPointerDown={event => {
      if (event.button !== 0) return;
      event.preventDefault(); dragging.current = true;
      strip.current?.setPointerCapture?.(event.pointerId);
      scrub.drag(indexAt(event));
    }}
    onPointerMove={event => { if (dragging.current) scrub.drag(indexAt(event)); }}
    onPointerUp={event => {
      if (!dragging.current) return;
      dragging.current = false; scrub.finish(indexAt(event));
      strip.current?.releasePointerCapture?.(event.pointerId);
    }}
    onPointerCancel={() => { dragging.current = false; scrub.cancel(); }}>
    {tape.segments.map(segment => {
      const { media } = segment;
      const whole = media.facts.duration.ticks / media.facts.duration.scale;
      const mark = (time: Media['mark_in']) => time && whole > 0 && !media.facts.video?.still ? Math.min(1, time.ticks / time.scale / whole) : null;
      const markIn = mark(media.mark_in);
      const markOut = mark(media.mark_out);
      return <div key={media.id} className="source-tape-media" data-tape-media={media.id} data-current={media.id === loaded || undefined}
        style={{ left: percent(segment.start, tape.seconds), width: percent(segment.seconds, tape.seconds) }} title={media.name}>
        <span className="source-tape-name">{media.name}</span>
        {markIn !== null && <span className="source-tape-mark" data-tape-mark="in" data-media={media.id} style={{ left: percent(markIn, 1) }} />}
        {markOut !== null && <span className="source-tape-mark" data-tape-mark="out" data-media={media.id} style={{ left: percent(markOut, 1) }} />}
        {markIn !== null && markOut !== null && markOut > markIn && <span className="source-tape-range" data-media={media.id}
          style={{ left: percent(markIn, 1), width: percent(markOut - markIn, 1) }} />}
      </div>;
    })}
    {head !== null && <span className="source-tape-head" data-test="tape-head" style={{ left: percent(head, tape.seconds) }} />}
  </div>;
}
