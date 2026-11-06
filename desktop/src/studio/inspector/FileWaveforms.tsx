import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { EngineBridge } from '../../bridge';
import type { Clip, Media, Timeline, Track } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { channelNames } from '../fairlight/ChannelLanes';
import { ChannelWaveform } from '../timeline/ChannelWaveform';
import { Waveforms } from '../timeline/waveforms';

/** Per-channel source views; timeline instances deliberately have no composite lane. */
export function FileWaveforms({ bridge, session, clip, track, media, timeline, selected, onSelect }: {
  bridge: Pick<EngineBridge, 'call' | 'onEvent'>; session: number; clip: Clip; track: Track;
  media?: Media; timeline: Timeline; selected: readonly number[]; onSelect: (index: number, extend: boolean) => void;
}) {
  const t = useFairlightText(), box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const waves = useMemo(() => new Waveforms(bridge, session), [bridge, session]);
  useEffect(() => { waves.connect(); return () => waves.disconnect(); }, [waves]);
  useLayoutEffect(() => {
    const element = box.current;
    if (!element) return;
    const measure = () => setWidth(element.clientWidth);
    measure();
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null;
    observer?.observe(element);
    return () => observer?.disconnect();
  }, []);
  return <div ref={box} className="file-waveforms" role="group" aria-label={t('Channels')}>
    {channelNames(track).map((name, index) => <button key={index} type="button" className="file-channel-waveform"
      aria-label={`${t('Channel')} ${index + 1}`} aria-pressed={selected.includes(index)}
      onClick={event => onSelect(index, event.ctrlKey || event.metaKey || event.shiftKey)}>
      <span className="file-channel-name">{name}</span>
      {width > 0 && <ChannelWaveform clip={clip} track={track} media={media} index={index} rate={timeline.rate}
        from={clip.start} to={clip.start + clip.length} scale={width / Math.max(1, clip.length)} waves={waves} />}
    </button>)}
  </div>;
}
