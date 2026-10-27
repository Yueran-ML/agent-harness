import { memo, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from 'react';
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

/** One media item of the list. It is drawn again only when what it shows changes. */
const Row = memo(function Row({ media, told, timelines, chosen, loaded, tabbable, onChoose, onLoad, onMakeAgain }: {
  media: Media;
  told: ProxyEvent | undefined;
  timelines: readonly { rate: Rate }[];
  chosen: boolean;
  loaded: boolean;
  tabbable: boolean;
  onChoose: (id: string) => void;
  onLoad: (id: string) => void;
  onMakeAgain: (id: string) => void;
}) {
  const words = useWords();
  const facts = useMemo(() => factsOf(words, media, timelines).join(' · '), [words, media, timelines]);
  return <li
    role="option"
    className="pool-item"
    aria-selected={chosen}
    aria-current={loaded || undefined}
    tabIndex={tabbable ? 0 : -1}
    title={media.path}
    onClick={() => onChoose(media.id)}
    onDoubleClick={() => onLoad(media.id)}
    data-test="pool-item"
    data-media={media.id}
  >
    {media.facts.video ? <Film size={15} aria-hidden="true" /> : <AudioLines size={15} aria-hidden="true" />}
    <span className="pool-words">
      <span className="pool-name" data-test="pool-name">{fileName(media.path)}</span>
      <span className="pool-facts" data-test="pool-facts">{facts}</span>
      <ProxyLine standing={proxyStanding(media, told)} onMakeAgain={() => onMakeAgain(media.id)} />
    </span>
  </li>;
});

/**
 * The media of the project: its name, what it holds and how its proxy stands. A click chooses
 * one, the arrows go up and down the list, and a double click or Enter shows it in the viewer.
 * The list follows the project as the engine tells it, whoever changed it. It is drawn again only
 * when the project, the proxies or what is chosen or loaded changes, not as the playhead moves.
 */
export const MediaPool = memo(function MediaPool({ media, timelines, proxies, loaded, onImport, onLoad, onMakeAgain }: {
  /** The media of the project, or null while there is no project to show. */
  media: readonly Media[] | null;
  timelines: readonly { rate: Rate }[];
  proxies: Readonly<Record<string, ProxyEvent>>;
  /** The media that the viewer shows, if any. */
  loaded: string | null;
  onImport: () => void;
  onLoad: (id: string) => void;
  onMakeAgain: (id: string) => void;
}) {
  const words = useWords();
  const list = useRef<HTMLUListElement>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const all = media ?? [];
  // What is chosen is what was picked, while it is there; the keyboard is on it, or on the first.
  const chosen = all.some(each => each.id === picked) ? picked : null;
  const tabbable = chosen ?? loaded ?? all[0]?.id ?? null;
  const failed = all.find(each => each.id === chosen && proxies[each.id]?.state === 'failed') ?? null;

  const move = (event: KeyboardEvent) => {
    if (event.key === 'Enter') {
      if (chosen === null) return;
      event.preventDefault();
      onLoad(chosen);
      return;
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    event.preventDefault();
    if (all.length === 0) return;
    const at = all.findIndex(each => each.id === (chosen ?? tabbable));
    const next = all[Math.max(0, Math.min(all.length - 1, at < 0 ? 0 : at + (event.key === 'ArrowDown' ? 1 : -1)))];
    setPicked(next.id);
    [...(list.current?.children ?? [])].find((row): row is HTMLElement => row instanceof HTMLElement && row.dataset.media === next.id)?.focus();
  };

  return <section className="pool" aria-label={words.label.studio.pool} data-test="pool">
    <header className="pool-head">
      <h2 className="overline">{words.label.studio.pool}</h2>
      <button type="button" className="btn btn-quiet" onClick={onImport} data-test="import">
        <Import size={14} aria-hidden="true" />{words.label.studio.import}
      </button>
    </header>
    {media !== null && media.length === 0 && <p className="caption pool-none" data-test="pool-none">{words.hint.studio.noMedia}</p>}
    {all.length > 0 && <ul ref={list} className="pool-list" role="listbox" aria-label={words.label.studio.pool} onKeyDown={move} data-test="pool-list">
      {all.map(each => <Row
        key={each.id}
        media={each}
        told={proxies[each.id]}
        timelines={timelines}
        chosen={chosen === each.id}
        loaded={loaded === each.id}
        tabbable={tabbable === each.id}
        onChoose={setPicked}
        onLoad={onLoad}
        onMakeAgain={onMakeAgain}
      />)}
    </ul>}
    {failed && <footer className="pool-foot">
      <button type="button" className="btn btn-quiet" onClick={() => onMakeAgain(failed.id)} data-test="make-proxy-again">
        <RotateCw size={14} aria-hidden="true" />{words.label.studio.makeProxyAgain}
      </button>
    </footer>}
  </section>;
});
