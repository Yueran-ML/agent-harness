import { memo, useContext, type ReactNode } from 'react';
import { useWords } from '../../app/locale';
import type { Clip, Media, Rate, Retime } from '../engine';
import { hasKeys } from '../inspector/clipKeys';
import { percentOf } from '../retime';
import { timecode } from '../timecode';
import { ClipKeyMarks } from './ClipKeyMarks';
import { spanStyle } from './geometry';
import { OpenRetimeControls } from './useRetimeControls';
import { Waveform } from './Waveform';
import type { Waveforms } from './waveforms';

/** A speed in hundredths of the speed of the source as a clip says it: to a tenth, with a minus where it plays backward. */
const said = (percent: number) => `${percent < 0 ? '−' : ''}${Number(Math.abs(percent).toFixed(1))}`;

/** The speeds of a clip with speed points in a few signs: the slowest and the fastest of its segments, in percent; a freeze is 0, and backward is below it. */
function speedRange(retime: Retime): string {
  const percents = retime.points.map(point => percentOf(point.speed));
  const low = Math.min(...percents);
  const high = Math.max(...percents);
  return low === high ? `${said(low)}%` : `${said(low)}–${said(high)}%`;
}

/**
 * The speeds of a clip with speed points, said at its lower right while its Retime Controls are
 * shut. Open, the bar says the speed of every segment, and the badge would lie on the speed of
 * the last one: it is not shown then.
 */
function RetimedSpeeds({ clip, retime }: { clip: string; retime: Retime }) {
  const open = useContext(OpenRetimeControls);
  return open?.has(clip) ? null : <span className="timeline-clip-speed" data-retimed="">{speedRange(retime)}</span>;
}

/**
 * A clip on its lane, placed by the style sheet from its frames (see `spanStyle`). `scale`, the
 * pixels of a frame, is needed only by the waveform of a clip of sound; a clip of video is not
 * given it, nor drawn again at a zoom.
 *
 * A clip of picture whose transform or crop has keys shows them (`ClipKeyMarks`) where the page
 * shows keys on its timeline. A clip without keys pays ten looks at its own settings for that as
 * it is drawn, and nothing else: no component, nothing read from the page.
 *
 * A clip with speed points says so where a clip of another speed says its speed: the slowest and
 * the fastest of its segments (`RetimedSpeeds`, which only such a clip mounts, and which says
 * nothing while the Retime Controls of the clip are open). The controls are not here: the timeline
 * lays them over the clips that they are open on (`RetimeBars`), so a clip without speed points
 * has no part of them, and mounts nothing for them.
 */
export const ClipView = memo(function ClipView({ clip, kind, media, rate, scale = 0, from, to, waves, selected = false, children, overlay }: {
  clip: Clip; kind: 'video' | 'audio'; media: Media | undefined; rate: Rate; scale?: number; from: number; to: number; waves: Waveforms;
  selected?: boolean;
  /** Page-specific drawing inside the same editable clip (Fairlight channel lanes, T19 overlays). */
  children?: ReactNode;
  overlay?: ReactNode;
}) {
  const words = useWords().label.studio.timeline;
  const name = clip.source.kind === 'media' ? media?.name ?? clip.source.media : clip.source.kind === 'title' ? clip.source.text : words.color;
  const changedSpeed = clip.reverse || clip.speed.num !== clip.speed.den;
  const type = clip.source.kind === 'media' ? kind : clip.source.kind;
  return <div className="timeline-clip" data-clip={clip.id} data-kind={type} data-enabled={clip.enabled} role="option" aria-selected={selected}
    style={spanStyle(clip.start, clip.length)}
    title={`${name} · ${timecode(clip.start, rate)} – ${timecode(clip.start + clip.length, rate)}`}>
    <span className="timeline-clip-name">{name}</span>
    {clip.retime ? <RetimedSpeeds clip={clip.id} retime={clip.retime} />
      : changedSpeed && <span className="timeline-clip-speed">{clip.reverse ? '−' : ''}{Number((clip.speed.num / clip.speed.den).toFixed(3))}×</span>}
    {children ?? (kind === 'audio' && media?.facts.audio && clip.source.kind === 'media' && <Waveform cache={waves} media={clip.source.media} clip={clip} rate={rate} scale={scale}
      from={Math.max(from, clip.start)} to={Math.min(to, clip.start + clip.length)} />)}
    {overlay}
    {/* The keys of its transform and crop: nothing is mounted for a clip that has none. */}
    {kind === 'video' && hasKeys(clip) && <ClipKeyMarks clip={clip} media={media} rate={rate} />}
  </div>;
});
