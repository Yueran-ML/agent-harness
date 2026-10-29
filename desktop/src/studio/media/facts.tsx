import type { MouseEvent } from 'react';
import { RotateCw } from 'lucide-react';
import { useWords } from '../../app/locale';
import type { Messages } from '../../messages/en';
import type { Media, ProxyEvent, Rate } from '../engine';
import { framesOf, rateOf, rateText, timecode } from '../timecode';

/** How the proxy of media stands: being made, made, failed, or none at all. */
export type ProxyStanding =
  | { kind: 'queued' }
  | { kind: 'making'; percent: number }
  | { kind: 'ready' }
  | { kind: 'failed'; message: string | null }
  | null;

/**
 * How the proxy of media stands, from what the engine told of it last (`media.proxy`) and the
 * proxy that the project has for it. Media that needs none has none, and nothing is said.
 */
export function proxyStanding(media: Media, told: ProxyEvent | undefined): ProxyStanding {
  if (told?.state === 'queued') return { kind: 'queued' };
  if (told?.state === 'running') return { kind: 'making', percent: Math.max(0, Math.min(100, Math.round((told.progress ?? 0) * 100))) };
  if (told?.state === 'failed') return { kind: 'failed', message: told.message ?? null };
  if (told?.state === 'done' || media.proxy) return { kind: 'ready' };
  return null;
}

/** What media holds, in a line: its size upright, its rate, how long it lasts and its depth, or that it is sound alone. */
export function factsOf(words: Messages, media: Media, timelines: readonly { rate: Rate }[]): string[] {
  const rate = rateOf(media, timelines);
  const lasts = timecode(framesOf(media.facts.duration, rate), rate);
  const video = media.facts.video;
  if (!video) return [words.label.studio.audioOnly, lasts];
  const turned = video.rotation === 90 || video.rotation === 270;
  const [width, height] = turned ? [video.height, video.width] : [video.width, video.height];
  return [`${width}×${height}`, words.label.studio.fps(rateText(video.rate)), lasts, words.label.studio.bits(video.bits)];
}

export function ProxyLine({ standing, onMakeAgain }: { standing: ProxyStanding; onMakeAgain: () => void }) {
  const words = useWords();
  if (standing === null) return null;
  const said = words.label.studio.proxy;
  if (standing.kind === 'failed') {
    // The button is for the mouse; the keyboard has the one under the list, for the media chosen.
    const again = (event: MouseEvent) => {
      event.stopPropagation();
      onMakeAgain();
    };
    return <span className="pool-proxy" data-tone="bad" data-test="pool-proxy">
      <span title={standing.message ?? words.hint.studio.proxyFailed}>{said.failed}</span>
      <button type="button" className="pool-again" tabIndex={-1} onClick={again} onDoubleClick={event => event.stopPropagation()} data-test="make-again">
        <RotateCw size={12} aria-hidden="true" />{words.label.studio.makeAgain}
      </button>
    </span>;
  }
  return <span className="pool-proxy" data-tone={standing.kind === 'ready' ? 'ok' : undefined} data-test="pool-proxy">
    {standing.kind === 'making' && <span className="pool-meter" aria-hidden="true"><span style={{ width: `${standing.percent}%` }} /></span>}
    {standing.kind === 'queued' ? said.queued : standing.kind === 'making' ? said.making(standing.percent) : said.ready}
  </span>;
}

