import { useEffect, useRef, useState, type PointerEvent } from 'react';
import type { AudioFade, Clip, Rate } from '../engine';
import { clamp } from '../audio/taper';
import { fadeValue, sampleTime, seconds } from './audioKeys';
import type { AudioGesture } from './useAudioGesture';
import { useFairlightText } from '../fairlight/words';

interface Props { clip: Clip; timeline: string; rate: Rate; sampleRate: number; scale: number; height: number; locked: boolean; edges: { in: boolean; out: boolean }; gesture: AudioGesture }
interface Drag { side: 'in' | 'out'; shape: boolean; x: number; y: number; width: number; height: number; initial: AudioFade; limit: number }
export function FadeHandles({ clip, timeline, rate, sampleRate, scale, height, locked, edges, gesture }: Props) {
  const t = useFairlightText();
  const drag = useRef<Drag | null>(null);
  const [preview, setPreview] = useState<{ side: 'in' | 'out'; value: AudioFade } | null>(null);
  useEffect(() => () => gesture.cancel(), []);
  const group = useRef<SVGGElement>(null);
  const width = Math.max(1, clip.length * scale), duration = clip.length * rate.den / rate.num;
  const fade = (side: 'in' | 'out'): AudioFade => gesture.active() && preview?.side === side ? preview.value
    : clip[side === 'in' ? 'audio_fade_in' : 'audio_fade_out'] ?? { length: sampleTime((side === 'in' ? clip.fade_in : clip.fade_out) * rate.den / rate.num, sampleRate) };
  const patch = (side: 'in' | 'out', value: AudioFade | null) => ({ op: 'set_clip' as const, timeline, id: clip.id, [side === 'in' ? 'audio_fade_in' : 'audio_fade_out']: value });
  const down = (event: PointerEvent<SVGGElement>, side: 'in' | 'out', shape: boolean) => {
    if (event.button !== 0) return;
    event.preventDefault(); event.stopPropagation(); group.current?.focus();
    if (locked || edges[side]) return;
    const initial = fade(side);
    // Preserve the old field as well: restoring null alone can otherwise revive a legacy fade.
    const field = side === 'in' ? 'audio_fade_in' : 'audio_fade_out';
    const legacy = side === 'in' ? 'fade_in' : 'fade_out';
    if (!gesture.start([{ ...patch(side, clip[field] ?? null), [legacy]: clip[legacy] }])) return;
    const box = event.currentTarget.ownerSVGElement!.getBoundingClientRect();
    drag.current = { side, shape, initial, x: event.clientX, y: event.clientY, width: box.width || width, height: box.height || 64,
      limit: Math.max(0, duration - seconds(fade(side === 'in' ? 'out' : 'in').length)) };
    group.current?.setPointerCapture?.(event.pointerId);
  };
  const move = (event: PointerEvent<SVGGElement>) => {
    const d = drag.current; if (!d) return;
    event.stopPropagation();
    if (!gesture.active()) { drag.current = null; setPreview(null); return; }
    const dx = event.clientX - d.x, dy = event.clientY - d.y;
    const value: AudioFade = d.shape ? { ...d.initial,
      curve: clamp((d.initial.curve ?? 0) - 2 * dy / d.height, -1, 1), skew: clamp((d.initial.skew ?? 0) + 2 * dx / d.width, -1, 1) }
      : { ...d.initial, length: { ticks: Math.min(Math.floor(d.limit * sampleRate), Math.round(clamp(seconds(d.initial.length) + (d.side === 'in' ? dx : -dx) / d.width * duration, 0, d.limit) * sampleRate)), scale: sampleRate } };
    setPreview({ side: d.side, value }); gesture.preview([patch(d.side, value)]);
  };
  const finish = (cancel: boolean) => { drag.current = null; setPreview(null); if (cancel) gesture.cancel(); else gesture.end(); };
  return <g ref={group} tabIndex={0} role="group" aria-label={t('Clip fades')} className="audio-fades"
    onPointerMove={move} onPointerUp={event => { event.stopPropagation(); if (drag.current) finish(false); }}
    onPointerCancel={event => { event.stopPropagation(); finish(true); }} onLostPointerCapture={() => { if (drag.current) finish(true); }}
    onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); finish(true); } }}>
    {(['in', 'out'] as const).map(side => {
      const value = fade(side), length = seconds(value.length), pixels = length / duration * width;
      const x = side === 'in' ? pixels : width - pixels;
      const curve = Array.from({ length: 25 }, (_, i) => { const fraction = i / 24;
        return `${i === 0 ? 'M' : 'L'}${side === 'in' ? fraction * pixels : width - fraction * pixels},${88 - 76 * fadeValue(fraction, value.curve, value.skew)}`; }).join(' ');
      return <g key={side}>
        {length > 0 && <><path d={curve} className="audio-fade-curve" />
          <ellipse data-testid={`fade-curve-${side}-${clip.id}`} cx={side === 'in' ? pixels / 2 : width - pixels / 2} cy={88 - 76 * fadeValue(.5, value.curve, value.skew)} rx={4} ry={4 * 100 / height}
            className="audio-fade-shape" onPointerDown={event => down(event, side, true)}
            onDoubleClick={event => { event.stopPropagation(); if (!locked && !edges[side]) gesture.commit([patch(side, { ...value, curve: 0, skew: 0 })]); }} />
        </>}
        <rect data-testid={`fade-${side}-${clip.id}`} x={clamp(x - 5, 0, Math.max(0, width - 10))} y={1} width={10} height={10 * 100 / height} className="audio-fade-handle" data-disabled={locked || edges[side]}
          onPointerDown={event => down(event, side, false)}
          onDoubleClick={event => { event.stopPropagation(); if (!locked && !edges[side]) gesture.commit([patch(side, { length: { ticks: 0, scale: sampleRate }, curve: 0, skew: 0 })]); }}>
          <title>{t(side === 'in' ? 'Fade In' : 'Fade Out')}</title>
        </rect>
        {drag.current?.side === side && gesture.active() && <rect className="audio-fade-shadow" x={x - scale / 2} width={scale} height={100} />}
      </g>;
    })}
  </g>;
}
