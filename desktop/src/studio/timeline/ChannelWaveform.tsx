import type { Clip, Media, Rate, Track } from '../engine';
import { useFairlightText } from '../fairlight/words';
import { Waveform } from './Waveform';
import type { Waveforms } from './waveforms';

/** The same source/channel interpretation in the timeline and the File inspector. */
export function channelWaveform(clip: Clip, track: Track, media: Media | undefined, index: number) {
  const map = clip.channels?.[index], source = map?.source;
  const automatic = clip.channels == null;
  const count = track.kind !== 'audio' ? 0 : track.format === 'mono' ? 1 : track.format === '5.1' ? 6 : track.format === 'adaptive' ? track.channels ?? 1 : 2;
  const remixed = automatic && (media?.facts.audio?.channels !== count
    || track.kind === 'audio' && track.format === '5.1' && track.order === 'film');
  const channel = Array.isArray(source) ? 0 : source ? source.channel : index;
  const stream = source && !Array.isArray(source) ? source.stream : undefined;
  const available = stream === undefined ? media?.facts.audio?.channels : media?.facts.audio?.streams?.find(each => each.index === stream)?.channels;
  const silent = map?.mute || map?.enabled === false || source === null || Array.isArray(source) && source.length === 0 || !automatic && !map;
  // An incomplete numeric draft has no level to preview yet.
  const level = map?.level_db ?? 0;
  return { channel, stream, mix: Array.isArray(source) ? source : undefined, gain: 10 ** ((Number.isFinite(level) ? level : 0) / 20),
    silent: !!silent || !automatic && available !== undefined && channel >= available,
    unavailable: remixed ? 'Remixed waveform unavailable' as const : null };
}

export function ChannelWaveform({ clip, track, media, index, rate, from, to, scale, waves, gain = 1 }: {
  clip: Clip; track: Track; media?: Media; index: number; rate: Rate; from: number; to: number;
  scale: number; waves: Waveforms; gain?: number;
}) {
  const t = useFairlightText(), drawing = channelWaveform(clip, track, media, index);
  if (drawing.silent) return null;
  if (drawing.unavailable) return <span className="timeline-wave-status">{t(drawing.unavailable)}</span>;
  return media && clip.source.kind === 'media' ? <Waveform cache={waves} media={media.id} clip={clip} rate={rate}
    scale={scale} from={Math.max(from, clip.start)} to={Math.min(to, clip.start + clip.length)}
    channel={drawing.channel} gain={gain * drawing.gain} stream={drawing.stream} mix={drawing.mix} /> : null;
}
