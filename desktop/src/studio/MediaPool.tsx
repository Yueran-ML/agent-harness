import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Grid2X2, Import, List, RotateCw, Trash2, X } from 'lucide-react';
import { useWords } from '../app/locale';
import type { EngineBridge } from '../bridge';
import { callEngine, EngineError, type Bin, type Media, type Op, type ProxyEvent, type Rate } from './engine';
import { Bins } from './media/Bins';
import { Clips } from './media/Clips';
import { Details } from './media/Details';
import { EMPTY_SELECTION, type Selection } from './media/selection';
import { Thumbnails } from './media/Thumbnails';
export { factsOf, proxyStanding } from './media/facts';

/** Media IDs and metadata are edited through one engine transaction; the original files stay where they are. */
export const MediaPool = memo(function MediaPool({ media, bins, timelines, proxies, loaded, bridge, session, onImport, onLoad, onMakeAgain, initialGrid = false, onBin }: {
  media: readonly Media[] | null; bins: readonly Bin[]; timelines: readonly { rate: Rate }[];
  proxies: Readonly<Record<string, ProxyEvent>>; loaded: string | null; bridge: Pick<EngineBridge, 'call'>; session: number;
  onImport: (bin: string | null) => void; onLoad: (id: string) => void; onMakeAgain: (id: string) => void;
  /** Whether it opens in the thumbnail view, as the Cut page's pool does. */
  initialGrid?: boolean;
  /** Told of the bin shown (null for the master bin), whose media the source tape of the Cut page plays. */
  onBin?: (bin: string | null) => void;
}) {
  const words = useWords();
  const labels = words.label.studio.media;
  const [bin, setBin] = useState<string | null>(null);
  const [selection, setSelection] = useState<Selection>(EMPTY_SELECTION);
  const [grid, setGrid] = useState(initialGrid);
  const [problem, setProblem] = useState<{ said: string; detail: string } | null>(null);
  const pending = useRef(false);
  const selectedBin = bins.some(each => each.id === bin) ? bin : null;
  useEffect(() => { onBin?.(selectedBin); }, [selectedBin, onBin]);
  const visible = useMemo(() => (media ?? []).filter(each => each.bin === selectedBin), [media, selectedBin]);
  const chosen = visible.filter(each => selection.ids.includes(each.id));
  const details = chosen.find(each => each.id === selection.focus) ?? chosen[0];
  const failed = chosen.find(each => proxies[each.id]?.state === 'failed');
  const thumbnails = useMemo(() => new Thumbnails(bridge), [bridge, session]);
  const edit = useCallback(async (ops: Op[]): Promise<boolean> => {
    if (ops.length === 0 || pending.current) return false;
    pending.current = true;
    setProblem(null);
    try {
      await callEngine(bridge, 'transaction.apply', { ops });
      return true;
    } catch (error) {
      const reason = error instanceof EngineError ? (error.data as { reason?: string } | null)?.reason : null;
      const said = reason === 'not_empty' ? words.hint.studio.binUsed
        : reason === 'in_use' ? words.hint.studio.mediaUsed : words.hint.studio.mediaEditFailed;
      setProblem({ said, detail: error instanceof Error ? error.message : String(error) });
      return false;
    } finally { pending.current = false; }
  }, [bridge, words]);
  const selectBin = (id: string | null) => { setBin(id); setSelection(EMPTY_SELECTION); };
  const move = (ids: string[], target: string | null) => {
    const moving = (media ?? []).filter(each => ids.includes(each.id) && each.bin !== target);
    // Media already in that bin does not move; the engine refuses a step that changes nothing.
    if (moving.length === 0) return;
    void edit(moving.map(each => ({ op: 'set_media', id: each.id, bin: target })));
  };
  return <section className="pool" aria-label={words.label.studio.pool} data-test="pool">
    <header className="pool-head">
      <h2 className="overline">{words.label.studio.pool}</h2>
      <div className="pool-toolbar">
        <button type="button" className="icon-btn" aria-label={labels.list} title={labels.list} aria-pressed={!grid} onClick={() => setGrid(false)}><List size={14} /></button>
        <button type="button" className="icon-btn" aria-label={labels.thumbnails} title={labels.thumbnails} aria-pressed={grid} onClick={() => setGrid(true)}><Grid2X2 size={14} /></button>
        <button type="button" className="btn btn-quiet" onClick={() => onImport(selectedBin)} data-test="import"><Import size={14} aria-hidden="true" />{words.label.studio.import}</button>
      </div>
    </header>
    {problem && <div className="pool-problem" role="alert"><p>{problem.said}<span className="caption">{problem.detail}</span></p>
      <button type="button" className="icon-btn" aria-label={words.label.close} onClick={() => setProblem(null)}><X size={14} /></button></div>}
    <div className="pool-browser">
      <Bins bins={bins} selected={selectedBin} onSelect={selectBin} onEdit={edit} onMove={move} session={session} />
      <div className="pool-clips">
        {media !== null && visible.length === 0 && <p className="caption pool-none" data-test="pool-none">{media.length === 0 ? words.hint.studio.noMedia : words.hint.studio.emptyBin}</p>}
        <Clips media={visible} timelines={timelines} proxies={proxies} loaded={loaded} selection={selection} onSelect={setSelection}
          onLoad={onLoad} onMakeAgain={onMakeAgain} onEdit={edit} session={session} thumbnails={grid ? thumbnails : null} />
      </div>
    </div>
    {chosen.length > 0 && <footer className="pool-foot">
      <span className="caption">{labels.selected(chosen.length)}</span>
      <button type="button" className="icon-btn" aria-label={labels.remove} title={labels.remove}
        onClick={() => void edit(chosen.map(each => ({ op: 'remove_media', id: each.id })))}><Trash2 size={14} /></button>
      {failed && <button type="button" className="btn btn-quiet" onClick={() => onMakeAgain(failed.id)} data-test="make-proxy-again"><RotateCw size={14} aria-hidden="true" />{words.label.studio.makeProxyAgain}</button>}
    </footer>}
    {details && <Details media={details} told={proxies[details.id]} timelines={timelines} onMakeAgain={() => onMakeAgain(details.id)} />}
  </section>;
});
