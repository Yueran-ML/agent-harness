import { useWords } from '../../app/locale';
import type { Media, ProxyEvent, Rate, Time } from '../engine';
import { framesOf, outFrame, rateOf, rateText, timecode } from '../timecode';
import { ProxyLine, proxyStanding } from './facts';

export function Details({ media, told, timelines, onMakeAgain }: { media: Media; told: ProxyEvent | undefined; timelines: readonly { rate: Rate }[]; onMakeAgain: () => void }) {
  const labels = useWords().label.studio;
  const words = labels.media;
  const { video, audio } = media.facts;
  const rate = rateOf(media, timelines);
  const at = (time: Time | null) => time ? timecode(framesOf(time, rate), rate) : '—';
  const turned = video?.rotation === 90 || video?.rotation === 270;
  const facts: [string, string][] = [
    [words.path, media.path], [words.duration, at(media.facts.duration)],
    ...(video ? [[words.videoCodec, video.codec], [words.dimensions, turned ? `${video.height}×${video.width}` : `${video.width}×${video.height}`],
      [words.frameRate, labels.fps(rateText(video.rate))], [words.depth, labels.bits(video.bits)]] as [string, string][] : []),
    ...(audio ? [[words.audioCodec, audio.codec], [words.sampleRate, `${audio.rate} Hz`], [words.channels, String(audio.channels)]] as [string, string][] : []),
    // The Out shown is the last frame marked: the mark is kept as the end of the range, after it.
    [words.markIn, at(media.mark_in)], [words.markOut, media.mark_out ? timecode(outFrame(framesOf(media.mark_out, rate)), rate) : '—'],
  ];
  const standing = proxyStanding(media, told);
  return <section className="pool-details" aria-label={words.details}>
    <h3>{media.name}</h3>
    <dl>{facts.map(([label, value]) => <div key={label}><dt>{label}</dt><dd title={value}>{value}</dd></div>)}</dl>
    {standing ? <ProxyLine standing={standing} onMakeAgain={onMakeAgain} /> : <p className="caption">{words.noProxy}</p>}
  </section>;
}
