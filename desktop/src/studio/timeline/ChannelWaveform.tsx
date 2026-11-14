import type { ChannelMap, Clip, Media, Rate, Track } from '../engine';
import { Waveform } from './Waveform';
import type { Waveforms } from './waveforms';

/** Explicit source maps are shared by pool channels and timeline instances. */
export function mappedWaveform(map: ChannelMap | undefined, media: Media | undefined) {
  const source = map?.source;
  const channel = map?.remix?.channel ?? (Array.isArray(source) ? 0 : source?.channel ?? 0);
  const stream = source && !Array.isArray(source) ? source.stream : undefined;
  const available = media?.facts.audio?.streams?.find(each => each.index === stream)?.channels;
  const level = map?.level_db ?? 0;
  return { channel, stream, mix: !map?.remix && Array.isArray(source) ? source : undefined, remix: map?.remix?.channels,
    gain: 10 ** ((Number.isFinite(level) ? level : 0) / 20),
    silent: !map || map.mute === true || map.enabled === false || !map.remix && (source == null
      || Array.isArray(source) && source.length === 0) || available !== undefined && channel >= available };
}

/** The same source/channel interpretation in the timeline and the File inspector. */
export function channelWaveform(clip: Clip, track: Track, media: Media | undefined, index: number) {
  if (clip.channels != null) return mappedWaveform(clip.channels[index], media);
  const count = track.kind !== 'audio' ? 0 : track.format === 'mono' ? 1 : track.format === '5.1' ? 6 : track.format === 'adaptive' ? track.channels ?? 1 : 2;
  const remix = media?.facts.audio?.channels !== count ? count : undefined;
  // Automatic playback supplies SMPTE; only the displayed Film lane order changes.
  const automaticChannel = track.kind === 'audio' && track.format === '5.1' && track.order === 'film' ? [0, 2, 1, 4, 5, 3][index] : index;
  return { channel: automaticChannel, stream: undefined, mix: undefined, remix, gain: 1, silent: false };
}

export function ChannelWaveform({ clip, track, media, index, rate, from, to, scale, waves, gain = 1 }: {
  clip: Clip; track: Track; media?: Media; index: number; rate: Rate; from: number; to: number;
  scale: number; waves: Waveforms; gain?: number;
}) {
  const drawing = channelWaveform(clip, track, media, index);
  if (drawing.silent) return null;
  return media && clip.source.kind === 'media' ? <Waveform cache={waves} media={media.id} clip={clip} rate={rate}
    scale={scale} from={Math.max(from, clip.start)} to={Math.min(to, clip.start + clip.length)}
    channel={drawing.channel} gain={gain * drawing.gain} stream={drawing.stream} mix={drawing.mix} remix={drawing.remix} /> : null;
}
