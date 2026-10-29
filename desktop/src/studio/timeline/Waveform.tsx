import { memo, useCallback, useMemo, useSyncExternalStore } from 'react';
import { useWords } from '../../app/locale';
import type { Clip, Rate } from '../engine';
import { sourceInterval, type PeakData, type Waveforms } from './waveforms';

function paths(data: PeakData, clip: Clip, rate: Rate, from: number, to: number, width: number): string[] {
  const columns = Math.max(1, Math.ceil(width / 2));
  return Array.from({ length: data.channels }, (_, channel) => {
    const half = 30 / data.channels;
    const center = (channel + 0.5) * 60 / data.channels + 2;
    let path = '';
    for (let column = 0; column < columns; column++) {
      const [a, b] = sourceInterval(clip, rate, from + (to - from) * column / columns, from + (to - from) * (column + 1) / columns);
      const [min, max] = data.range(a * data.rate, b * data.rate, channel);
      const x = (column + 0.5) * width / columns;
      path += `M${x.toFixed(1)},${(center - max / 127 * half).toFixed(1)}V${(center - min / 127 * half).toFixed(1)}`;
    }
    return path;
  });
}

export const Waveform = memo(function Waveform({ cache, media, clip, rate, from, to, scale }: {
  cache: Waveforms; media: string; clip: Clip; rate: Rate; from: number; to: number; scale: number;
}) {
  const words = useWords().label.studio.timeline;
  const subscribe = useCallback((listener: () => void) => cache.subscribe(media, listener), [cache, media]);
  const snapshot = useCallback(() => cache.get(media), [cache, media]);
  const state = useSyncExternalStore(subscribe, snapshot, snapshot);
  const width = Math.max(1, (to - from) * scale);
  const drawn = useMemo(() => state.state === 'ready' ? paths(state.data, clip, rate, from, to, width) : [], [state, clip, rate, from, to, width]);
  if (state.state !== 'ready') return <span className="timeline-wave-status" title={state.state === 'failed' ? state.message : undefined}>
    {state.state === 'failed' ? words.waveFailed : words.wavePending}
  </span>;
  return <svg className="timeline-waveform" viewBox={`0 0 ${width} 64`} preserveAspectRatio="none" aria-hidden="true"
    style={{ left: (from - clip.start) * scale, width }} data-waveform={media}>
    {drawn.map((path, index) => <path key={index} d={path} />)}
  </svg>;
});
