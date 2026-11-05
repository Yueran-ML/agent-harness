import { useFairlightText } from './words';
import type { Track } from '../engine';
import { Waveform } from '../timeline/Waveform';
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
  const automatic = clip.channels == null;
  // Auto and the default waveform both use libav's default layout. Different channel
  // counts need a remix; a Film track also cannot identify its lanes by SMPTE indexes.
  const remixed = automatic && (media?.facts.audio?.channels !== names.length
    || (track.kind === 'audio' && track.format === '5.1' && track.order === 'film'));
  return <div className="fairlight-channel-lanes" style={{ gridTemplateRows: `repeat(${names.length}, minmax(0, 1fr))` }}>
    {names.map((name, index) => {
      const map = clip.channels?.[index]; const source = map?.source;
      // Explicit picks use their native stream order; combined sources need mixed peaks.
      const channel = source && !Array.isArray(source) ? source.channel : index;
      const stream = source && !Array.isArray(source) ? source.stream : undefined;
      const count = stream === undefined ? media?.facts.audio?.channels : media?.facts.audio?.streams?.find(each => each.index === stream)?.channels;
      const silence = map?.mute || map?.enabled === false || source === null || (!automatic && !map);
      const unavailable = remixed ? t('Remixed waveform unavailable') : Array.isArray(source) ? t('Mixed waveform unavailable') : null;
      const missingChannel = !automatic && count !== undefined && channel >= count;
      return <div className="fairlight-channel-lane" data-channel={index} key={index}><small>{name}{Array.isArray(source) ? ` · ${t('mix')}` : ''}</small>
        {!silence && unavailable && <span className="timeline-wave-status">{unavailable}</span>}
        {!silence && !missingChannel && !unavailable && media && clip.source.kind === 'media' && <Waveform cache={waves} media={media.id} clip={clip} rate={rate} scale={scale}
          from={Math.max(from, clip.start)} to={Math.min(to, clip.start + clip.length)} channel={channel} gain={gain} stream={stream} />}
      </div>;
    })}
  </div>;
}
