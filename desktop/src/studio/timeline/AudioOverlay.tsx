import type { EngineBridge } from '../../bridge';
import { useLayoutEffect, useRef, useState } from 'react';
import type { TrackClipDrawing } from './TrackRow';
import type { Marks } from './snap';
import { useAudioGesture } from './useAudioGesture';
import { VolumeLine, type AudioClipboard } from './VolumeLine';
import { FadeHandles } from './FadeHandles';

export interface AudioOverlayContext {
  bridge: Pick<EngineBridge, 'call'>; timeline: string; revision: number; session: number; frame: number;
  sampleRate: number; snapping: boolean; marks: Marks; clipboard: AudioClipboard; gainLine: boolean;
  onSeek: (frame: number, scrubbing: boolean) => void; onFailure: (error: unknown) => void;
}
export function AudioOverlay({ drawing, context }: { drawing: TrackClipDrawing; context: AudioOverlayContext }) {
  const { clip, track, media, rate, scale } = drawing;
  const svg = useRef<SVGSVGElement>(null);
  const [height, setHeight] = useState(64);
  useLayoutEffect(() => {
    const element = svg.current;
    if (!element) return;
    const measure = () => { if (element.clientHeight > 0) setHeight(element.clientHeight); };
    measure(); const observer = new ResizeObserver(measure); observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const gesture = useAudioGesture({ ...context, identity: `${context.timeline}/${clip.id}`, locked: track.locked });
  if (track.kind !== 'audio') return null;
  const edges = { in: track.transitions.some(each => each.right === clip.id), out: track.transitions.some(each => each.left === clip.id) };
  const fades = <FadeHandles clip={clip} timeline={context.timeline} rate={rate} sampleRate={context.sampleRate} scale={scale} height={height} locked={track.locked} edges={edges} gesture={gesture} />;
  return <svg ref={svg} className="audio-overlay" data-testid={`audio-overlay-${clip.id}`} viewBox={`0 0 ${Math.max(1, clip.length * scale)} 100`} preserveAspectRatio="none">
    {context.gainLine ? <VolumeLine {...context} clip={clip} rate={rate} scale={scale} height={height} from={drawing.from} to={drawing.to} sampleRate={media?.facts.audio?.rate ?? context.sampleRate}
      locked={track.locked} gesture={gesture}>{fades}</VolumeLine> : fades}
  </svg>;
}
