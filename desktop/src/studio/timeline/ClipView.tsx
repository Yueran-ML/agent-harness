import { memo, type ReactNode } from 'react';
import { useWords } from '../../app/locale';
import type { Clip, Media, Rate } from '../engine';
import { timecode } from '../timecode';
import { spanStyle } from './geometry';
import { Waveform } from './Waveform';
import type { Waveforms } from './waveforms';

/**
 * A clip on its lane, placed by the style sheet from its frames (see `spanStyle`). `scale`, the
 * pixels of a frame, is needed only by the waveform of a clip of sound; a clip of video is not
 * given it, nor drawn again at a zoom.
 */
export const ClipView = memo(function ClipView({ clip, kind, media, rate, scale = 0, from, to, waves, selected = false, children }: {
  clip: Clip; kind: 'video' | 'audio'; media: Media | undefined; rate: Rate; scale?: number; from: number; to: number; waves: Waveforms;
  selected?: boolean;
  /** Page-specific drawing inside the same editable clip (Fairlight channel lanes, T19 overlays). */
  children?: ReactNode;
}) {
  const words = useWords().label.studio.timeline;
  const name = clip.source.kind === 'media' ? media?.name ?? clip.source.media : clip.source.kind === 'title' ? clip.source.text : words.color;
  const changedSpeed = clip.reverse || clip.speed.num !== clip.speed.den;
  const type = clip.source.kind === 'media' ? kind : clip.source.kind;
  return <div className="timeline-clip" data-clip={clip.id} data-kind={type} data-enabled={clip.enabled} role="option" aria-selected={selected}
    style={spanStyle(clip.start, clip.length)}
    title={`${name} · ${timecode(clip.start, rate)} – ${timecode(clip.start + clip.length, rate)}`}>
    <span className="timeline-clip-name">{name}</span>
    {changedSpeed && <span className="timeline-clip-speed">{clip.reverse ? '−' : ''}{Number((clip.speed.num / clip.speed.den).toFixed(3))}×</span>}
    {children ?? (kind === 'audio' && media?.facts.audio && clip.source.kind === 'media' && <Waveform cache={waves} media={clip.source.media} clip={clip} rate={rate} scale={scale}
      from={Math.max(from, clip.start)} to={Math.min(to, clip.start + clip.length)} />)}
  </div>;
});
