import { memo } from 'react';
import { useWords } from '../../app/locale';
import type { Clip, Media, Rate } from '../engine';
import { timecode } from '../timecode';
import { Waveform } from './Waveform';
import type { Waveforms } from './waveforms';

export const ClipView = memo(function ClipView({ clip, kind, media, rate, scale, from, to, waves, selected = false }: {
  clip: Clip; kind: 'video' | 'audio'; media: Media | undefined; rate: Rate; scale: number; from: number; to: number; waves: Waveforms;
  selected?: boolean;
}) {
  const words = useWords().label.studio.timeline;
  const name = clip.source.kind === 'media' ? media?.name ?? clip.source.media : clip.source.kind === 'title' ? clip.source.text : words.color;
  const changedSpeed = clip.reverse || clip.speed.num !== clip.speed.den;
  const type = clip.source.kind === 'media' ? kind : clip.source.kind;
  return <div className="timeline-clip" data-clip={clip.id} data-kind={type} data-enabled={clip.enabled} role="option" aria-selected={selected}
    style={{ left: clip.start * scale, width: clip.length * scale }}
    title={`${name} · ${timecode(clip.start, rate)} – ${timecode(clip.start + clip.length, rate)}`}>
    <span className="timeline-clip-name">{name}</span>
    {changedSpeed && <span className="timeline-clip-speed">{clip.reverse ? '−' : ''}{Number((clip.speed.num / clip.speed.den).toFixed(3))}×</span>}
    {kind === 'audio' && media?.facts.audio && clip.source.kind === 'media' && <Waveform cache={waves} media={clip.source.media} clip={clip} rate={rate} scale={scale}
      from={Math.max(from, clip.start)} to={Math.min(to, clip.start + clip.length)} />}
  </div>;
});
