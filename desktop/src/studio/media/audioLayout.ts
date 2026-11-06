import type { AudioStream, ChannelMap, ChannelSource, Media, MediaAudioTrack, Track, TrackFormat } from '../engine';

export const audioSources = (media: Media): ChannelSource[] =>
  (media.facts.audio?.streams ?? []).flatMap(stream =>
    Array.from({ length: stream.channels }, (_, channel) => ({ stream: stream.index, channel })));

export const channelSources = (source: ChannelMap['source']): ChannelSource[] =>
  source ? Array.isArray(source) ? source : [source] : [];

/** Retain every automatic output exactly, including the display order of a Film track. */
export function automaticChannels(track: Track): ChannelMap[] {
  if (track.kind !== 'audio') return [];
  const count = formatChannels(track.format ?? 'stereo', track.channels ?? 1);
  return Array.from({ length: count }, (_, index) => ({ source: null, enabled: true, mute: false, level_db: 0,
    remix: { channels: count, channel: track.format === '5.1' && track.order === 'film' ? [0, 2, 1, 4, 5, 3][index] : index },
  }));
}

export const channelKey = (map: ChannelMap, streams: readonly AudioStream[]) =>
  map.remix ? `remix:${map.remix.channels}:${map.remix.channel}` : sourceKey(map.source, streams);

export function channelChoice(value: string, streams: readonly AudioStream[]): Partial<ChannelMap> {
  if (value.startsWith('remix:')) {
    const [, channels, channel] = value.split(':').map(Number);
    return { source: null, remix: { channels, channel } };
  }
  return { source: sourceChoice(value, streams), remix: undefined };
}

export function sourceKey(source: ChannelMap['source'], streams: readonly AudioStream[]): string {
  if (!source || Array.isArray(source) && source.length === 0) return 'none';
  if (!Array.isArray(source)) return `${source.stream}:${source.channel}`;
  const whole = streams.find(stream => source.length === stream.channels && source.every((item, i) => item.stream === stream.index && item.channel === i));
  return whole ? `mix:${whole.index}` : `custom:${JSON.stringify(source)}`;
}

export function sourceChoice(value: string, streams: readonly AudioStream[]): ChannelMap['source'] {
  if (value === 'none') return null;
  const [stream, channel] = value.split(':');
  return stream === 'mix'
    ? Array.from({ length: streams.find(item => item.index === Number(channel))?.channels ?? 0 }, (_, i) => ({ stream: Number(channel), channel: i }))
    : { stream: Number(stream), channel: Number(channel) };
}

export const formatChannels = (format: TrackFormat, adaptive: number): number =>
  format === 'mono' ? 1 : format === 'stereo' ? 2 : format === '5.1' ? 6 : adaptive;

export function sourceTrack(media: Media, format: TrackFormat, offset = 0, adaptive = 1): MediaAudioTrack {
  const sources = audioSources(media);
  return { name: '', format, order: 'smpte', channels: Array.from({ length: formatChannels(format, adaptive) }, (_, i) =>
    ({ source: sources[offset + i] ?? null, enabled: true, mute: false, level_db: 0 })) };
}

export function sourceLayout(media: Media): MediaAudioTrack[] {
  if (media.audio_config) return structuredClone(media.audio_config);
  let offset = 0;
  return (media.facts.audio?.streams ?? []).map(stream => {
    const format = stream.channels === 1 ? 'mono' : stream.channels === 2 ? 'stereo' : stream.channels === 6 ? '5.1' : 'adaptive';
    const row = sourceTrack(media, format, offset, stream.channels);
    offset += stream.channels;
    return row;
  });
}

/** A standard format keeps the surviving channels' source choices and trims. */
export function standardLayout(media: Media, format: TrackFormat): MediaAudioTrack[] {
  const old = sourceLayout(media), channels = old.flatMap(track => track.channels);
  const row = sourceTrack(media, format, 0, Math.max(1, Math.min(24, channels.length)));
  row.name = old[0]?.name ?? '';
  if (format === '5.1' && old.length === 1 && old[0].format === '5.1') row.order = old[0].order;
  row.channels = row.channels.map((channel, i) => channels[i] ?? channel);
  return [row];
}

/** Batch edits address embedded channel positions, not another file's libav stream IDs. */
export function mapLayout(config: MediaAudioTrack[], reference: Media, target: Media): MediaAudioTrack[] | null {
  const original = audioSources(reference), destination = audioSources(target);
  const copy = structuredClone(config);
  for (const row of copy) {
    for (const channel of row.channels) {
      const mapped: ChannelSource[] = [];
      for (const source of channelSources(channel.source)) {
        const at = original.findIndex(item => item.stream === source.stream && item.channel === source.channel);
        if (at < 0 || !destination[at]) return null;
        mapped.push(destination[at]);
      }
      channel.source = mapped.length === 0 ? null : mapped.length === 1 ? mapped[0] : mapped;
    }
  }
  return copy;
}
