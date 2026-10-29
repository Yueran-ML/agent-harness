import { memo, useMemo } from 'react';
import type { Rate } from '../engine';
import { timecode } from '../timecode';
import { HEADER_WIDTH, rulerSteps } from './geometry';

/**
 * The ticks of the ruler between `from` and `to` (frames): a large tick with its timecode (written
 * by the style sheet from `data-label`, one element a tick) at each label, and the
 * small ticks as one strip drawn by the style sheet, a line at each step. Drawn again only where
 * the window, the zoom or the rate changes, never as the playhead moves.
 */
export const RulerTicks = memo(function RulerTicks({ rate, scale, from, to }: { rate: Rate; scale: number; from: number; to: number }) {
  const { major, minor } = useMemo(() => rulerSteps(scale, rate), [scale, rate]);
  const first = Math.max(0, Math.ceil(from / minor) * minor);
  const labels = useMemo(() => {
    const frames = [];
    for (let frame = Math.max(0, Math.ceil(from / major) * major); frame < to; frame += major) frames.push(frame);
    return frames;
  }, [from, to, major]);
  return <>
    {to > first && <span className="timeline-ruler-minor" aria-hidden="true"
      style={{ left: HEADER_WIDTH + first * scale, width: (to - first) * scale, backgroundSize: `${minor * scale}px 100%` }} />}
    {labels.map(frame => <span key={frame} className="timeline-tick" data-major="true" data-label={timecode(frame, rate)} style={{ left: HEADER_WIDTH + frame * scale }} />)}
  </>;
});
