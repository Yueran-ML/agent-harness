import { useFairlightText } from './words';
import type { Track } from '../engine';
import { ChannelWaveform } from '../timeline/ChannelWaveform';
import type { TrackClipDrawing } from '../timeline/TrackRow';
export function channelNames(track: Track): string[] {
  if (track.kind !== 'audio') return [];
  if (track.format === 'mono') return ['M'];
  if (track.format === '5.1') return track.order === 'film' ? ['L', 'C', 'R', 'Ls', 'Rs', 'LFE'] : ['L', 'R', 'C', 'LFE', 'Ls', 'Rs'];
  if (track.format === 'adaptive') return Array.from({ length: track.channels ?? 1 }, (_, at) => String(at + 1));
  return ['L', 'R'];
}
export function ChannelLanes({ clip, track, media, rate, scale, from, to, waves, gain }: TrackClipDrawing & { gain: number }) {
  const t = useFairlightText();
  const names = channelNames(track);
  return <div className="fairlight-channel-lanes" style={{ gridTemplateRows: `repeat(${names.length}, minmax(0, 1fr))` }}>
    {names.map((name, index) => {
      const source = clip.channels?.[index]?.source;
      return <div className="fairlight-channel-lane" data-channel={index} key={index}><small>{name}{Array.isArray(source) ? ` · ${t('mix')}` : ''}</small>
        <ChannelWaveform clip={clip} track={track} media={media} index={index} rate={rate} scale={scale} from={from} to={to} waves={waves} gain={gain} />
      </div>;
    })}
  </div>;
}
