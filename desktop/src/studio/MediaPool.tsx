import { useState, type KeyboardEvent } from 'react';
import { AudioLines, Film, Import, RotateCw } from 'lucide-react';
import { useWords } from '../app/locale';
import type { Messages } from '../messages/en';
import type { Media, ProxyEvent, Rate } from './engine';
import { fileName, framesOf, rateOf, rateText, timecode } from './timecode';

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

function ProxyLine({ standing, onMakeAgain }: { standing: ProxyStanding; onMakeAgain: () => void }) {
  const words = useWords();
  if (standing === null) return null;
  const said = words.label.studio.proxy;
  if (standing.kind === 'failed') {
    return <span className="pool-proxy" data-tone="bad" data-test="pool-proxy">
      <span title={standing.message ?? words.hint.studio.proxyFailed}>{said.failed}</span>
      <button type="button" className="pool-again" onClick={onMakeAgain} data-test="make-again">
        <RotateCw size={12} aria-hidden="true" />{words.label.studio.makeAgain}
      </button>
    </span>;
  }
  return <span className="pool-proxy" data-tone={standing.kind === 'ready' ? 'ok' : undefined} data-test="pool-proxy">
    {standing.kind === 'making' && <span className="pool-meter" aria-hidden="true"><span style={{ width: `${standing.percent}%` }} /></span>}
    {standing.kind === 'queued' ? said.queued : standing.kind === 'making' ? said.making(standing.percent) : said.ready}
  </span>;
}

/**
 * The media of the project: its name, what it holds and how its proxy stands. A click chooses
 * one, and a double click or Enter shows it in the viewer. The list follows the project as the
 * engine tells it, whoever changed it.
 */
export function MediaPool({ media, timelines, proxies, loaded, onImport, onLoad, onMakeAgain }: {
  media: readonly Media[];
  timelines: readonly { rate: Rate }[];
  proxies: Readonly<Record<string, ProxyEvent>>;
  /** The media that the viewer shows, if any. */
  loaded: string | null;
  onImport: () => void;
  onLoad: (id: string) => void;
  onMakeAgain: (id: string) => void;
}) {
  const words = useWords();
  const [chosen, setChosen] = useState<string | null>(null);
  const enter = (event: KeyboardEvent, id: string) => {
    if (event.key !== 'Enter') return;
    event.preventDefault();
    onLoad(id);
  };
  return <section className="pool" aria-label={words.label.studio.pool} data-test="pool">
    <header className="pool-head">
      <h2 className="overline">{words.label.studio.pool}</h2>
      <button type="button" className="btn btn-quiet" onClick={onImport} data-test="import">
        <Import size={14} aria-hidden="true" />{words.label.studio.import}
      </button>
    </header>
    {media.length === 0
      ? <p className="caption pool-none" data-test="pool-none">{words.hint.studio.noMedia}</p>
      : <ul className="pool-list">
        {media.map(each => <li key={each.id} className="pool-row" data-test="pool-item" data-media={each.id}>
          <button
            type="button"
            className="pool-item"
            aria-pressed={chosen === each.id}
            aria-current={loaded === each.id || undefined}
            title={each.path}
            onClick={() => setChosen(each.id)}
            onDoubleClick={() => onLoad(each.id)}
            onKeyDown={event => enter(event, each.id)}
          >
            {each.facts.video ? <Film size={15} aria-hidden="true" /> : <AudioLines size={15} aria-hidden="true" />}
            <span className="pool-words">
              <span className="pool-name" data-test="pool-name">{fileName(each.path)}</span>
              <span className="pool-facts" data-test="pool-facts">{factsOf(words, each, timelines).join(' · ')}</span>
            </span>
          </button>
          <ProxyLine standing={proxyStanding(each, proxies[each.id])} onMakeAgain={() => onMakeAgain(each.id)} />
        </li>)}
      </ul>}
  </section>;
}
