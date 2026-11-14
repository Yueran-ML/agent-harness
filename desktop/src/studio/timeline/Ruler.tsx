import { memo, type PointerEvent } from 'react';
import { useWords } from '../../app/locale';
import type { Rate } from '../engine';
import { timecode } from '../timecode';
import { HEADER_WIDTH } from './geometry';
import { RulerTicks } from './RulerTicks';

/**
 * The ruler, a slider of the playhead. What changes as the playhead moves is the slider's value and
 * its head; the ticks are drawn apart (`RulerTicks`), for the window of the tracks.
 */
export const Ruler = memo(function Ruler({ rate, scale, from, to, frame, last, onDown, onSeek }: {
  rate: Rate; scale: number; from: number; to: number; frame: number; last: number;
  onDown: (event: PointerEvent) => void; onSeek: (frame: number, scrubbing: boolean) => void;
}) {
  const words = useWords().label.studio.timeline;
  return <div className="timeline-ruler" role="slider" tabIndex={0} aria-label={words.position}
    aria-valuemin={0} aria-valuemax={last} aria-valuenow={frame} aria-valuetext={timecode(frame, rate)} onPointerDown={onDown}
    onKeyDown={event => {
      let next: number;
      if (event.key === 'ArrowLeft') next = frame - 1;
      else if (event.key === 'ArrowRight') next = frame + 1;
      else if (event.key === 'Home') next = 0;
      else if (event.key === 'End') next = last;
      else return;
      event.preventDefault(); event.stopPropagation(); onSeek(Math.max(0, Math.min(last, next)), false);
    }}>
    <RulerTicks rate={rate} scale={scale} from={from} to={to} />
    <span className="timeline-ruler-head" style={{ left: HEADER_WIDTH + frame * scale }} aria-hidden="true" />
  </div>;
});
